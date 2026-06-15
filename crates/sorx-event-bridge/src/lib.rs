//! Event bridge: consume `greentic.sorla.request.v1` from NATS, invoke the local
//! sorx runtime via the `SorxInvoker` seam, and publish `greentic.sorla.response.v1`
//! echoing the correlation id.
//!
//! The `RuntimeDispatch*` structs below MIRROR the canonical contract in
//! `greentic-types::runtime_dispatch` (kept in sync by hand; duplicated because
//! greentic-sorx pins a different greentic-types lineage than the runner).

use std::sync::Arc;

use anyhow::Result;
use async_nats::HeaderMap;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const REQUEST_SUBJECT: &str = "greentic.sorla.request.v1";
const RESPONSE_SUBJECT: &str = "greentic.sorla.response.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchMode {
    Await,
    FireAndForget,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDispatchRequest {
    pub target: String,
    pub operation: String,
    pub mode: DispatchMode,
    pub input: Value,
    pub deadline_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeDispatchResponse {
    pub ok: bool,
    pub output: Value,
    #[serde(default)]
    pub events: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<DispatchError>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DispatchError {
    pub code: String,
    pub message: String,
}

/// Result of invoking the local sorx runtime.
pub struct InvokeOutcome {
    pub ok: bool,
    pub output: Value,
    pub events: Vec<Value>,
}

/// Seam over the actual sorx invocation. Production impl wraps sorx-core's EndpointRouter.
#[async_trait]
pub trait SorxInvoker: Send + Sync {
    async fn invoke(
        &self,
        tenant: &str,
        env: &str,
        target: &str,
        operation: &str,
        input: Value,
        idempotency_key: Option<&str>,
    ) -> Result<InvokeOutcome>;
}

/// Invoke and build the response (pure-ish: no NATS I/O). Errors map to an error response.
pub async fn build_response(
    invoker: Arc<dyn SorxInvoker>,
    tenant: &str,
    env: &str,
    idempotency_key: Option<&str>,
    req: RuntimeDispatchRequest,
) -> RuntimeDispatchResponse {
    match invoker
        .invoke(
            tenant,
            env,
            &req.target,
            &req.operation,
            req.input,
            idempotency_key,
        )
        .await
    {
        Ok(outcome) => RuntimeDispatchResponse {
            ok: outcome.ok,
            output: outcome.output,
            events: outcome.events,
            error: None,
        },
        Err(error) => RuntimeDispatchResponse {
            ok: false,
            output: Value::Null,
            events: vec![],
            error: Some(DispatchError {
                code: "invoke_failed".into(),
                message: error.to_string(),
            }),
        },
    }
}

/// Handle one request message end-to-end: decode, invoke, publish response.
pub async fn handle_message(
    client: &async_nats::Client,
    invoker: Arc<dyn SorxInvoker>,
    msg: async_nats::Message,
) -> Result<()> {
    let headers = msg.headers.as_ref();
    // HeaderMap::get returns Option<&HeaderValue>; HeaderValue::as_str() gives &str.
    let get_header = |name: &str| -> Option<String> {
        headers
            .and_then(|header_map| header_map.get(name))
            .map(|value| value.as_str().to_string())
    };

    let correlation = get_header("Greentic-Correlation-Id");
    let tenant = get_header("Greentic-Tenant").unwrap_or_default();
    let env = get_header("Greentic-Env").unwrap_or_else(|| "default".to_string());

    let req: RuntimeDispatchRequest = serde_json::from_slice(&msg.payload)?;
    let resp = build_response(invoker, &tenant, &env, correlation.as_deref(), req).await;

    let mut out_headers = HeaderMap::new();
    if let Some(correlation_value) = correlation.as_deref() {
        out_headers.insert("Greentic-Correlation-Id", correlation_value);
    }
    out_headers.insert("Greentic-Tenant", tenant.as_str());
    out_headers.insert("Greentic-Env", env.as_str());

    let response_bytes = serde_json::to_vec(&resp)?;
    client
        .publish_with_headers(RESPONSE_SUBJECT, out_headers, response_bytes.into())
        .await?;
    Ok(())
}

/// Subscribe to the request subject and serve forever (one task per message).
pub async fn run_bridge(client: async_nats::Client, invoker: Arc<dyn SorxInvoker>) -> Result<()> {
    use futures_util::StreamExt;
    let mut subscriber = client.subscribe(REQUEST_SUBJECT).await?;
    while let Some(msg) = subscriber.next().await {
        let client = client.clone();
        let invoker = invoker.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_message(&client, invoker, msg).await {
                tracing::error!(%error, "sorx event bridge failed to handle request");
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    struct StubInvoker {
        seen: Mutex<Vec<(String, String, serde_json::Value)>>,
    }

    #[async_trait::async_trait]
    impl SorxInvoker for StubInvoker {
        async fn invoke(
            &self,
            _tenant: &str,
            _env: &str,
            target: &str,
            operation: &str,
            input: serde_json::Value,
            _idempotency_key: Option<&str>,
        ) -> anyhow::Result<InvokeOutcome> {
            self.seen.lock().unwrap().push((
                target.to_string(),
                operation.to_string(),
                input.clone(),
            ));
            Ok(InvokeOutcome {
                ok: true,
                output: json!({"echo": input}),
                events: vec![json!({"e": 1})],
            })
        }
    }

    #[test]
    fn request_decodes_and_maps_to_invocation() {
        let body = serde_json::to_vec(&RuntimeDispatchRequest {
            target: "dep-1".into(),
            operation: "create".into(),
            mode: DispatchMode::Await,
            input: json!({"a": 1}),
            deadline_ms: Some(1000),
        })
        .unwrap();
        let req: RuntimeDispatchRequest = serde_json::from_slice(&body).unwrap();
        assert_eq!(req.operation, "create");
        assert_eq!(req.mode, DispatchMode::Await);
    }

    #[tokio::test]
    async fn handle_invokes_and_builds_response() {
        let invoker = Arc::new(StubInvoker {
            seen: Mutex::new(vec![]),
        });
        let req = RuntimeDispatchRequest {
            target: "dep-1".into(),
            operation: "create".into(),
            mode: DispatchMode::Await,
            input: json!({"a": 1}),
            deadline_ms: None,
        };
        let resp = build_response(invoker.clone(), "t1", "default", Some("corr-9"), req).await;
        assert!(resp.ok);
        assert_eq!(resp.output["echo"], json!({"a": 1}));
        assert_eq!(resp.events.len(), 1);
        assert_eq!(invoker.seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn invoke_error_maps_to_error_response() {
        struct FailInvoker;

        #[async_trait::async_trait]
        impl SorxInvoker for FailInvoker {
            async fn invoke(
                &self,
                _tenant: &str,
                _env: &str,
                _target: &str,
                _operation: &str,
                _input: serde_json::Value,
                _idempotency_key: Option<&str>,
            ) -> anyhow::Result<InvokeOutcome> {
                Err(anyhow::anyhow!("boom"))
            }
        }

        let req = RuntimeDispatchRequest {
            target: "d".into(),
            operation: "x".into(),
            mode: DispatchMode::Await,
            input: json!(null),
            deadline_ms: None,
        };
        let resp = build_response(Arc::new(FailInvoker), "t1", "default", Some("c"), req).await;
        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "invoke_failed");
    }
}
