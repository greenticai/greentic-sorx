//! Production [`sorx_event_bridge::SorxInvoker`] backed by the real sorx
//! runtime.
//!
//! The event bridge consumes `greentic.sorla.request.v1` from NATS and needs to
//! invoke a local endpoint. [`CoreSorxInvoker`] wraps an [`Arc<SorxRuntime>`]
//! (obtained from the running [`crate::http_runtime::HttpRuntime`]) and maps a
//! dispatch request onto an [`EndpointInvocation`].
//!
//! Mapping decisions (documented for future maintainers):
//! - `target`    -> [`EndpointInvocation::endpoint_id`]
//! - `operation` -> [`EndpointInvocation::operation_id`]
//! - `tenant`    -> [`EndpointInvocation::tenant_id`]
//! - `env`       -> currently unused by the runtime invocation (the runtime is
//!   already bound to a single environment via its config); kept in the trait
//!   signature for parity with the wire contract.
//! - `caller`    -> a fixed service identity (`subject = "event-bridge"`,
//!   `roles = ["local"]`), mirroring the default roles the HTTP request path
//!   assigns to an unauthenticated local call (see `header_caller_roles`).
//! - `source`    -> [`InvocationSource::Direct`] (the dispatch did not arrive
//!   over the sorx HTTP surface nor the MCP surface).

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use greentic_sorx_core::{
    CallerContext, EndpointInvocation, EndpointResult, EndpointStatus, InvocationSource,
    SorxRuntime,
};

use sorx_event_bridge::{InvokeOutcome, SorxInvoker};

/// Service identity subject used for all event-bridge invocations.
const EVENT_BRIDGE_SUBJECT: &str = "event-bridge";
/// Default policy role for event-bridge invocations (matches the HTTP path's
/// fallback role for unauthenticated local calls).
const EVENT_BRIDGE_ROLE: &str = "local";

/// Production [`SorxInvoker`] wrapping the live sorx runtime.
pub struct CoreSorxInvoker {
    runtime: Arc<SorxRuntime>,
}

impl CoreSorxInvoker {
    /// Build an invoker over a shared runtime handle (see
    /// [`crate::http_runtime::HttpRuntime::runtime_handle`]).
    pub fn new(runtime: Arc<SorxRuntime>) -> Self {
        Self { runtime }
    }
}

/// Map a runtime [`EndpointResult`] onto the bridge's transport-agnostic
/// [`InvokeOutcome`].
///
/// `ok` is `true` only when the endpoint completed successfully
/// ([`EndpointStatus::Ok`], [`EndpointStatus::Created`], or
/// [`EndpointStatus::Deleted`] — all terminal success states). Soft failures
/// ([`EndpointStatus::Denied`], [`EndpointStatus::ApprovalRequired`],
/// [`EndpointStatus::NotFound`]) yield `ok = false` while still forwarding the
/// runtime's `output` and `events` so the caller can surface the reason.
fn outcome_from_result(result: EndpointResult) -> InvokeOutcome {
    let ok = matches!(
        result.status,
        EndpointStatus::Ok | EndpointStatus::Created | EndpointStatus::Deleted
    );
    let events = result
        .events
        .iter()
        .map(|event| serde_json::to_value(event).unwrap_or(Value::Null))
        .collect();
    InvokeOutcome {
        ok,
        output: result.output,
        events,
    }
}

#[async_trait]
impl SorxInvoker for CoreSorxInvoker {
    async fn invoke(
        &self,
        tenant: &str,
        _env: &str,
        target: &str,
        operation: &str,
        input: Value,
        idempotency_key: Option<&str>,
    ) -> Result<InvokeOutcome> {
        let invocation = EndpointInvocation {
            tenant_id: tenant.to_string(),
            endpoint_id: target.to_string(),
            operation_id: operation.to_string(),
            input,
            caller: CallerContext {
                subject: EVENT_BRIDGE_SUBJECT.to_string(),
                roles: vec![EVENT_BRIDGE_ROLE.to_string()],
            },
            idempotency_key: idempotency_key.map(str::to_string),
            source: InvocationSource::Direct,
        };

        // `SorxRuntime::invoke` is synchronous and may perform blocking store
        // I/O, so run it on the blocking pool to avoid stalling the async
        // reactor that drives the NATS subscription.
        let runtime = self.runtime.clone();
        let result = tokio::task::spawn_blocking(move || runtime.invoke(invocation))
            .await
            .context("sorx invoke task panicked")?
            .map_err(|err| anyhow::anyhow!("sorx endpoint invocation failed: {err}"))?;

        Ok(outcome_from_result(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_sorx_core::SorxEvent;
    use serde_json::json;

    fn sample_event() -> SorxEvent {
        SorxEvent {
            schema: "greentic.sorx.event.v1".to_string(),
            event_type: "record.created".to_string(),
            tenant_id: "tenant-1".to_string(),
            endpoint_id: "endpoint-1".to_string(),
            operation_id: "create".to_string(),
            provider_binding: "store".to_string(),
        }
    }

    #[test]
    fn ok_status_maps_to_ok_true() {
        let result = EndpointResult {
            status: EndpointStatus::Ok,
            output: json!({"id": "abc"}),
            events: vec![sample_event()],
        };
        let outcome = outcome_from_result(result);
        assert!(outcome.ok);
        assert_eq!(outcome.output, json!({"id": "abc"}));
        assert_eq!(outcome.events.len(), 1);
        assert_eq!(outcome.events[0]["event_type"], json!("record.created"));
    }

    #[test]
    fn created_and_deleted_map_to_ok_true() {
        for status in [EndpointStatus::Created, EndpointStatus::Deleted] {
            let label = format!("{status:?}");
            let outcome = outcome_from_result(EndpointResult {
                status,
                output: Value::Null,
                events: vec![],
            });
            assert!(outcome.ok, "{label} should be ok");
        }
    }

    #[test]
    fn denied_status_maps_to_ok_false_but_keeps_output_and_events() {
        let result = EndpointResult {
            status: EndpointStatus::Denied,
            output: json!({"reason": "policy"}),
            events: vec![sample_event()],
        };
        let outcome = outcome_from_result(result);
        assert!(!outcome.ok);
        assert_eq!(outcome.output, json!({"reason": "policy"}));
        assert_eq!(outcome.events.len(), 1);
    }

    #[test]
    fn approval_required_and_not_found_map_to_ok_false() {
        for status in [EndpointStatus::ApprovalRequired, EndpointStatus::NotFound] {
            let label = format!("{status:?}");
            let outcome = outcome_from_result(EndpointResult {
                status,
                output: Value::Null,
                events: vec![],
            });
            assert!(!outcome.ok, "{label} should be a soft failure");
        }
    }
}
