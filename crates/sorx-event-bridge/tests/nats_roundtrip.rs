//! Gated NATS round-trip integration test for `sorx-event-bridge`.
//!
//! Self-skips when `GREENTIC_TEST_NATS_URL` is unset — no broker required
//! for CI. Set the env var to a live broker to run the full round-trip:
//!
//! ```text
//! GREENTIC_TEST_NATS_URL=nats://127.0.0.1:4222 cargo test -p sorx-event-bridge --test nats_roundtrip
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::json;
use sorx_event_bridge::{
    DispatchMode, InvokeOutcome, RuntimeDispatchRequest, RuntimeDispatchResponse, SorxInvoker,
    run_bridge,
};
use tokio::time::{Duration, timeout};

const REQUEST_SUBJECT: &str = "greentic.sorla.request.v1";
const RESPONSE_SUBJECT: &str = "greentic.sorla.response.v1";
const NATS_URL_ENV: &str = "GREENTIC_TEST_NATS_URL";

/// Echo stub: returns `{ "echo": <input> }` so the test can verify the
/// payload was carried through the NATS transport layer unmodified.
struct EchoStubInvoker;

#[async_trait]
impl SorxInvoker for EchoStubInvoker {
    async fn invoke(
        &self,
        _tenant: &str,
        _env: &str,
        _target: &str,
        _operation: &str,
        input: serde_json::Value,
        _idempotency_key: Option<&str>,
    ) -> anyhow::Result<InvokeOutcome> {
        Ok(InvokeOutcome {
            ok: true,
            output: json!({ "echo": input }),
            events: vec![],
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn nats_roundtrip_sorla_request_response() {
    // Self-skip when no broker is configured.
    let nats_url = match std::env::var(NATS_URL_ENV) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            println!("SKIP nats_roundtrip: {NATS_URL_ENV} not set — skipping (no broker)");
            return;
        }
    };

    // Connect two independent clients: one for the bridge, one for the test harness.
    let bridge_client = async_nats::connect(&nats_url)
        .await
        .expect("connect bridge client to NATS");
    let harness_client = async_nats::connect(&nats_url)
        .await
        .expect("connect harness client to NATS");

    // Subscribe to the response subject BEFORE publishing so we don't miss the reply.
    let mut response_subscriber = harness_client
        .subscribe(RESPONSE_SUBJECT)
        .await
        .expect("subscribe to response subject");

    // Spawn the bridge in the background; it will serve until the task is dropped.
    let bridge_handle = tokio::spawn(run_bridge(bridge_client, Arc::new(EchoStubInvoker)));

    // Give the bridge a moment to register its subscription.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Build the request payload.
    let request_input = json!({ "action": "deploy", "env": "staging" });
    let dispatch_request = RuntimeDispatchRequest {
        target: "sorla-deployment-service".into(),
        operation: "deploy".into(),
        mode: DispatchMode::Await,
        input: request_input.clone(),
        deadline_ms: Some(5_000),
    };
    let request_bytes: bytes::Bytes = serde_json::to_vec(&dispatch_request)
        .expect("serialize request")
        .into();

    // Attach required headers including the correlation id we want echoed back.
    let mut request_headers = async_nats::HeaderMap::new();
    request_headers.insert("Greentic-Correlation-Id", "corr-e2e-1");
    request_headers.insert("Greentic-Tenant", "t1");
    request_headers.insert("Greentic-Env", "default");

    harness_client
        .publish_with_headers(REQUEST_SUBJECT, request_headers, request_bytes)
        .await
        .expect("publish request");
    harness_client.flush().await.expect("flush publish");

    // Await the response with a generous timeout so slow CI machines don't flake.
    let response_message = timeout(Duration::from_secs(5), response_subscriber.next())
        .await
        .expect("timed out waiting for response (5s)")
        .expect("response subscriber closed before receiving a message");

    // Verify the correlation id header was echoed back.
    let response_headers = response_message
        .headers
        .as_ref()
        .expect("response message must carry headers");
    let echoed_correlation_id = response_headers
        .get("Greentic-Correlation-Id")
        .expect("Greentic-Correlation-Id header missing from response")
        .as_str();
    assert_eq!(
        echoed_correlation_id, "corr-e2e-1",
        "correlation id must be echoed verbatim"
    );

    // Verify the response body.
    let dispatch_response: RuntimeDispatchResponse =
        serde_json::from_slice(&response_message.payload)
            .expect("deserialize RuntimeDispatchResponse");
    assert!(
        dispatch_response.ok,
        "response must indicate success (ok: true)"
    );
    assert_eq!(
        dispatch_response.output["echo"], request_input,
        "output.echo must mirror the original input"
    );

    // Clean up: abort the bridge task (it loops indefinitely by design).
    bridge_handle.abort();
}
