use std::collections::BTreeMap;
use std::sync::Arc;

use greentic_sorx_core::{
    EndpointRouter, EndpointStatus, LocalAutoApproveBroker, LocalDenyBroker, MemoryAuditSink,
    MemoryStoreProvider, PolicyConfig, PolicyEngine, PolicyMode, ProviderRegistry, RiskLevel,
    SorxRuntime, default_start_schema, invocation, normalize_start_answers,
    runtime_config_from_answers, runtime_pack,
};
use serde_json::{Value, json};

fn gateway() -> Value {
    json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            endpoint("tenant.create.low", "create", "low"),
            endpoint("tenant.create.medium", "create", "medium"),
            endpoint("tenant.create.high", "create", "high"),
            endpoint("tenant.create.critical", "create", "critical")
        ]
    })
}

fn endpoint(id: &str, operation: &str, risk: &str) -> Value {
    json!({
        "endpoint_id": id,
        "operation_id": id,
        "operation": operation,
        "method": "POST",
        "path": format!("/v1/{id}"),
        "entity": "Tenant",
        "collection": "tenants",
        "provider_binding": "store",
        "risk": risk,
        "input_schema": {
            "type": "object",
            "required": ["id", "name", "active"],
            "properties": {
                "id": { "type": "string" },
                "name": { "type": "string" },
                "active": { "type": "boolean" }
            }
        }
    })
}

fn answers() -> Value {
    json!({
        "tenant": { "tenant_id": "tenant-a" },
        "server": { "public_base_url": "http://127.0.0.1:8787" },
        "providers": { "store": { "kind": "memory", "config_ref": "providers.memory.local" } },
        "policy": { "approvals": {} },
        "audit": {},
        "deployment": { "tenant_id": "tenant-a", "sor_name": "landlord", "environment": "local" },
        "exposure": {},
        "ghcr": {}
    })
}

fn runtime() -> SorxRuntime {
    runtime_with_policy(PolicyEngine::default())
}

fn runtime_with_policy(policy: PolicyEngine) -> SorxRuntime {
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("landlord", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_store("store", Arc::new(MemoryStoreProvider::new()));
    SorxRuntime::new(runtime_pack("landlord", "0.1.0"), config, router, providers)
        .with_policy(policy)
}

fn input(id: &str) -> Value {
    json!({ "id": id, "name": "Acme", "active": true })
}

#[test]
fn low_and_medium_risk_execute_by_default() {
    let runtime = runtime();
    let low = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create.low",
            "tenant.create.low",
            input("tenant-low"),
        ))
        .unwrap();
    assert_eq!(low.status, EndpointStatus::Created);

    let medium = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create.medium",
            "tenant.create.medium",
            input("tenant-medium"),
        ))
        .unwrap();
    assert_eq!(medium.status, EndpointStatus::Created);
}

#[test]
fn high_risk_requires_approval_by_default_without_provider_call() {
    let runtime = runtime();
    let pending = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create.high",
            "tenant.create.high",
            input("tenant-high"),
        ))
        .unwrap();
    assert_eq!(pending.status, EndpointStatus::ApprovalRequired);
    assert_eq!(pending.output["status"], "approval_required");

    let missing = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create.low",
            "tenant.create.low",
            input("tenant-high"),
        ))
        .unwrap();
    assert_eq!(missing.output["id"], "tenant-high");
}

#[test]
fn critical_risk_is_denied_by_default_without_provider_call() {
    let runtime = runtime();
    let denied = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create.critical",
            "tenant.create.critical",
            input("tenant-critical"),
        ))
        .unwrap();
    assert_eq!(denied.status, EndpointStatus::Denied);
    assert_eq!(denied.output["status"], "denied");

    let created = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create.low",
            "tenant.create.low",
            input("tenant-critical"),
        ))
        .unwrap();
    assert_eq!(created.status, EndpointStatus::Created);
}

#[test]
fn auto_approval_broker_allows_high_risk_when_configured() {
    let policy = PolicyEngine::new(PolicyConfig {
        approvals: BTreeMap::from([
            (RiskLevel::Low, PolicyMode::Auto),
            (RiskLevel::Medium, PolicyMode::Auto),
            (RiskLevel::High, PolicyMode::RequireApproval),
            (RiskLevel::Critical, PolicyMode::Deny),
        ]),
    });
    let runtime =
        runtime_with_policy(policy).with_approval_broker(Arc::new(LocalAutoApproveBroker));
    let result = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create.high",
            "tenant.create.high",
            input("tenant-approved"),
        ))
        .unwrap();
    assert_eq!(result.status, EndpointStatus::Created);
}

#[test]
fn deny_broker_denies_high_risk_when_approval_is_required() {
    let runtime = runtime().with_approval_broker(Arc::new(LocalDenyBroker));
    let result = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create.high",
            "tenant.create.high",
            input("tenant-denied"),
        ))
        .unwrap();
    assert_eq!(result.status, EndpointStatus::Denied);
    assert_eq!(result.output["status"], "denied");
}

#[test]
fn audit_events_are_emitted_in_expected_order() {
    let audit = MemoryAuditSink::new();
    let runtime = runtime().with_audit_sink(Arc::new(audit.clone()));
    let mut invocation = invocation(
        "tenant-a",
        "tenant.create.low",
        "tenant.create.low",
        input("tenant-audit"),
    );
    invocation.idempotency_key = Some("audit-key".to_string());
    runtime.invoke(invocation).unwrap();

    let events = audit.events().unwrap();
    let names = events
        .iter()
        .map(|event| event.event.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "sorx.endpoint.invoked",
            "sorx.policy.decided",
            "sorx.provider.operation.started",
            "sorx.provider.operation.completed",
            "sorx.endpoint.completed"
        ]
    );
    assert!(events.iter().all(|event| event.idempotency_key_present));
}

#[test]
fn audit_events_do_not_include_request_body_by_default() {
    let audit = MemoryAuditSink::new();
    let runtime = runtime().with_audit_sink(Arc::new(audit.clone()));
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create.low",
            "tenant.create.low",
            json!({
                "id": "tenant-audit-body",
                "name": "Acme",
                "active": true,
                "password": "never-log-this"
            }),
        ))
        .unwrap();

    let encoded = serde_json::to_string(&audit.events().unwrap()).unwrap();
    assert!(!encoded.contains("never-log-this"));
    assert!(!encoded.contains("password"));
}

#[test]
fn idempotency_key_is_scoped_to_operation() {
    let runtime = runtime();
    let mut first = invocation(
        "tenant-a",
        "tenant.create.low",
        "tenant.create.low",
        input("tenant-one"),
    );
    first.idempotency_key = Some("same-key".to_string());
    let mut second = first.clone();
    second.input = input("tenant-two");
    let first = runtime.invoke(first).unwrap();
    let second = runtime.invoke(second).unwrap();
    assert_eq!(first.output["id"], "tenant-one");
    assert_eq!(second.output["id"], "tenant-one");
}

#[test]
fn strict_router_fails_missing_risk_metadata_on_mutation() {
    let gateway = json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [{
            "endpoint_id": "tenant.create",
            "operation_id": "tenant.create",
            "operation": "create",
            "method": "POST",
            "path": "/v1/tenants",
            "entity": "Tenant",
            "collection": "tenants",
            "provider_binding": "store"
        }]
    });
    let err = EndpointRouter::from_agent_gateway_strict(&gateway).unwrap_err();
    assert_eq!(err.code, "risk_missing");
}
