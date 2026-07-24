use std::sync::Arc;

use serde_json::{Map, Value};

use crate::{
    AuditSink, ControlDecision, ObserverEvent, RiskLevel, RuntimeExtensionAdapter,
    RuntimeExtensionBinding, RuntimeExtensionRegistry, SorxAuditEvent, SorxResult,
    redact_audit_value,
};

/// `pack_ref` advertised by the built-in audit observer.
pub const NATIVE_AUDIT_PACK_REF: &str = "greentic.sorx.audit.v1";

/// Built-in observer extension: records `pre_call`/`post_call`/`call_failed`/
/// `control_denied` observer events to the runtime's [`AuditSink`] as
/// [`SorxAuditEvent`]s. It is a companion to the invoke-time audit stream and is
/// necessarily coarser: `ObserverEvent` does not carry the endpoint's
/// operation/risk, so `operation_id` mirrors the route and `risk` is `Low`; the
/// full call context is preserved (redacted) under `details`.
pub struct NativeAuditObserver {
    audit_sink: Arc<dyn AuditSink>,
    pack: String,
    version: String,
}

impl std::fmt::Debug for NativeAuditObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeAuditObserver")
            .field("pack", &self.pack)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl NativeAuditObserver {
    pub fn new(
        audit_sink: Arc<dyn AuditSink>,
        pack: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            audit_sink,
            pack: pack.into(),
            version: version.into(),
        }
    }

    fn audit_event(
        &self,
        subscription: &str,
        event: &ObserverEvent,
        raw: &Value,
    ) -> SorxAuditEvent {
        let ctx = &event.context;
        let decision = event
            .control_decision
            .as_ref()
            .map(|d| control_action_label(&d.action).to_string())
            .or_else(|| event.status.clone());

        // Start from the raw `context` object (not the typed `StackCallContext`)
        // so extension-supplied fields the typed struct doesn't model (e.g.
        // `api_key`) are still captured — and still redacted — instead of
        // being silently dropped by `serde_json::from_value`.
        let mut details = match raw.get("context") {
            Some(Value::Object(map)) => map.clone(),
            _ => Map::new(),
        };
        details.insert("event_type".into(), Value::String(event.event_type.clone()));
        let details = match redact_audit_value(Value::Object(details)) {
            Value::Object(map) => map,
            _ => Map::new(),
        };

        SorxAuditEvent {
            event: format!("observer.{subscription}"),
            pack: self.pack.clone(),
            version: self.version.clone(),
            tenant_id: ctx.tenant_id.clone(),
            endpoint_id: ctx.route_id.clone(),
            operation_id: ctx.route_id.clone(),
            risk: RiskLevel::Low,
            caller_id: ctx.actor.clone(),
            decision,
            duration_ms: event.duration_ms,
            idempotency_key_present: false,
            details,
        }
    }
}

/// Builds a registry of the built-in native extension adapters. Phase 1
/// registers the audit observer under [`NATIVE_AUDIT_PACK_REF`]. New built-ins
/// are added here.
pub fn native_extension_registry(
    audit_sink: Arc<dyn AuditSink>,
    pack: impl Into<String>,
    version: impl Into<String>,
) -> RuntimeExtensionRegistry {
    RuntimeExtensionRegistry::new().with_adapter(
        NATIVE_AUDIT_PACK_REF,
        Arc::new(NativeAuditObserver::new(audit_sink, pack, version)),
    )
}

fn control_action_label(action: &crate::ControlDecisionAction) -> &'static str {
    match action {
        crate::ControlDecisionAction::Allow => "allow",
        crate::ControlDecisionAction::Deny => "deny",
        crate::ControlDecisionAction::AllowWithPatch => "allow_with_patch",
    }
}

impl RuntimeExtensionAdapter for NativeAuditObserver {
    fn observe(
        &self,
        subscription: &str,
        _binding: &RuntimeExtensionBinding,
        event: &Value,
    ) -> SorxResult<()> {
        let typed: ObserverEvent = serde_json::from_value(event.clone())
            .map_err(|err| crate::SorxError::new("native_audit_event_invalid", err.to_string()))?;
        self.audit_sink
            .emit(self.audit_event(subscription, &typed, event))
    }

    fn control(
        &self,
        _hook: &str,
        _binding: &RuntimeExtensionBinding,
        _request: &Value,
        _response: Option<&Value>,
    ) -> SorxResult<ControlDecision> {
        Ok(ControlDecision::allow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BoundObserverHook, ExtensionFailMode, MemoryAuditSink, ObserverHook, RiskLevel,
        RuntimeExtensionAdapter, RuntimeExtensionBinding, RuntimeExtensions,
    };
    use serde_json::json;
    use std::sync::Arc;

    fn binding() -> RuntimeExtensionBinding {
        RuntimeExtensionBinding {
            id: "audit".into(),
            contract: "greentic.cap.extension.observer.v1".into(),
            pack_ref: NATIVE_AUDIT_PACK_REF.into(),
            fail_mode: ExtensionFailMode::Open,
        }
    }

    fn observer_event(event_type: &str) -> serde_json::Value {
        json!({
            "event_type": event_type,
            "context": {
                "environment_id": "env", "runtime_id": "rt", "tenant_id": "acme",
                "deployment_id": "dep", "stack_id": "landlord", "route_id": "record_rent_payment",
                "call_id": "call-1", "trace_id": "trace-1", "actor": "alice",
                "api_key": "sk-secret-value"
            },
            "status": "ok",
            "duration_ms": 7
        })
    }

    #[test]
    fn observe_records_structured_audit_event() {
        let sink = MemoryAuditSink::new();
        let observer = NativeAuditObserver::new(Arc::new(sink.clone()), "landlord", "1.0.0");

        observer
            .observe(
                "post_call",
                &binding(),
                &observer_event("stack.call.completed"),
            )
            .unwrap();

        let events = sink.events().unwrap();
        assert_eq!(events.len(), 1);
        let ev = &events[0];
        assert_eq!(ev.event, "observer.post_call");
        assert_eq!(ev.pack, "landlord");
        assert_eq!(ev.version, "1.0.0");
        assert_eq!(ev.tenant_id, "acme");
        assert_eq!(ev.endpoint_id, "record_rent_payment");
        assert_eq!(ev.caller_id, "alice");
        assert_eq!(ev.decision.as_deref(), Some("ok"));
        assert_eq!(ev.duration_ms, Some(7));
        assert_eq!(ev.risk, RiskLevel::Low);
        // secret-like context fields are redacted in details
        assert_eq!(ev.details["api_key"], json!("[REDACTED]"));
        assert_eq!(ev.details["call_id"], json!("call-1"));
    }

    #[test]
    fn control_defaults_to_allow() {
        let sink = MemoryAuditSink::new();
        let observer = NativeAuditObserver::new(Arc::new(sink), "p", "1");
        let decision = observer
            .control("pre_call", &binding(), &json!({}), None)
            .unwrap();
        assert!(matches!(
            decision.action,
            crate::ControlDecisionAction::Allow
        ));
    }

    fn subscribe(extensions: &mut RuntimeExtensions, subscription: &str) {
        extensions
            .observer
            .subscriptions
            .entry(subscription.to_string())
            .or_default()
            .push(binding());
    }

    fn typed_event(event_type: &str) -> crate::ObserverEvent {
        serde_json::from_value(observer_event(event_type)).unwrap()
    }

    #[test]
    fn bound_observer_dispatches_declared_subscriptions_to_audit_sink() {
        let sink = MemoryAuditSink::new();
        let registry = native_extension_registry(Arc::new(sink.clone()), "landlord", "1.0.0");

        let mut extensions = RuntimeExtensions::default();
        subscribe(&mut extensions, "pre_call");
        subscribe(&mut extensions, "post_call");

        let hook = BoundObserverHook::new(extensions, registry);
        hook.observe(&typed_event("stack.call.started")).unwrap();
        hook.observe(&typed_event("stack.call.completed")).unwrap();

        let events = sink.events().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, "observer.pre_call");
        assert_eq!(events[1].event, "observer.post_call");
    }

    #[test]
    fn bound_observer_ignores_undeclared_subscriptions() {
        let sink = MemoryAuditSink::new();
        let registry = native_extension_registry(Arc::new(sink.clone()), "landlord", "1.0.0");
        // No subscriptions declared.
        let hook = BoundObserverHook::new(RuntimeExtensions::default(), registry);
        hook.observe(&typed_event("stack.call.completed")).unwrap();
        assert!(sink.events().unwrap().is_empty());
    }
}
