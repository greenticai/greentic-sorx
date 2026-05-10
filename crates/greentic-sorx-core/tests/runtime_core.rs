use std::sync::Arc;

use greentic_sorx_core::{
    EndpointRouter, EndpointStatus, MemoryStoreProvider, ProviderRegistry, SorxRuntime,
    default_start_schema, invocation, normalize_start_answers, runtime_config_from_answers,
    runtime_pack,
};
use serde_json::{Value, json};

fn gateway() -> Value {
    json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            {
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "operation": "create",
                "method": "POST",
                "path": "/v1/tenants",
                "entity": "Tenant",
                "collection": "tenants",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["id", "name", "active"],
                    "properties": {
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "active": { "type": "boolean" }
                    }
                }
            },
            {
                "endpoint_id": "tenant.get",
                "operation_id": "tenant.get",
                "operation": "get",
                "method": "GET",
                "path": "/v1/tenants/{id}",
                "entity": "Tenant",
                "collection": "tenants",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["id"],
                    "properties": {
                        "id": { "type": "string" }
                    }
                }
            },
            {
                "endpoint_id": "tenant.update",
                "operation_id": "tenant.update",
                "operation": "update",
                "method": "PATCH",
                "path": "/v1/tenants/{id}",
                "entity": "Tenant",
                "collection": "tenants",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["id", "patch"],
                    "properties": {
                        "id": { "type": "string" },
                        "patch": { "type": "object" }
                    }
                }
            },
            {
                "endpoint_id": "tenant.query",
                "operation_id": "tenant.query",
                "operation": "query",
                "method": "POST",
                "path": "/v1/tenants/query",
                "entity": "Tenant",
                "collection": "tenants",
                "provider_binding": "store"
            }
        ]
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
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("landlord", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_store("store", Arc::new(MemoryStoreProvider::new()));
    SorxRuntime::new(runtime_pack("landlord", "0.1.0"), config, router, providers)
}

#[test]
fn endpoint_router_builds_from_gateway_metadata() {
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let endpoint = router.endpoint("tenant.create").unwrap();
    assert_eq!(endpoint.operation_id, "tenant.create");
    assert_eq!(endpoint.collection, "tenants");
    assert_eq!(endpoint.provider_binding, "store");
}

#[test]
fn create_and_get_tenant_operation() {
    let runtime = runtime();
    let created = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-1", "name": "Acme", "active": true }),
        ))
        .unwrap();
    assert_eq!(created.status, EndpointStatus::Created);
    assert_eq!(created.output["id"], "tenant-1");

    let fetched = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.get",
            "tenant.get",
            json!({ "id": "tenant-1" }),
        ))
        .unwrap();
    assert_eq!(fetched.status, EndpointStatus::Ok);
    assert_eq!(fetched.output["data"]["name"], "Acme");
}

#[test]
fn update_and_query_active_tenants() {
    let runtime = runtime();
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-1", "name": "Acme", "active": true }),
        ))
        .unwrap();
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-2", "name": "Dormant", "active": false }),
        ))
        .unwrap();
    let updated = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.update",
            "tenant.update",
            json!({ "id": "tenant-2", "patch": { "active": true } }),
        ))
        .unwrap();
    assert_eq!(updated.output["version"], 2);

    let queried = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.query",
            "tenant.query",
            json!({ "filter": { "active": true } }),
        ))
        .unwrap();
    assert_eq!(queried.output["records"].as_array().unwrap().len(), 2);
    assert_eq!(queried.output["records"][0]["id"], "tenant-1");
    assert_eq!(queried.output["records"][1]["id"], "tenant-2");
}

#[test]
fn missing_provider_binding_fails_clearly() {
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("landlord", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let runtime = SorxRuntime::new(
        runtime_pack("landlord", "0.1.0"),
        config,
        router,
        ProviderRegistry::new(),
    );

    let err = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-1", "name": "Acme", "active": true }),
        ))
        .unwrap_err();
    assert_eq!(err.code, "provider_missing");
    assert!(err.message.contains("store"));
}

#[test]
fn unknown_endpoint_fails_clearly() {
    let err = runtime()
        .invoke(invocation("tenant-a", "missing", "missing", json!({})))
        .unwrap_err();
    assert_eq!(err.code, "unknown_endpoint");
}

#[test]
fn invalid_input_fails_before_provider_execution() {
    let runtime = runtime();
    let err = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-1", "name": "Acme" }),
        ))
        .unwrap_err();
    assert_eq!(err.code, "invalid_input");
    assert_eq!(err.path.as_deref(), Some("active"));
}

#[test]
fn idempotency_key_prevents_duplicate_create() {
    let runtime = runtime();
    let mut first = invocation(
        "tenant-a",
        "tenant.create",
        "tenant.create",
        json!({ "id": "tenant-1", "name": "Acme", "active": true }),
    );
    first.idempotency_key = Some("create-acme".to_string());
    let mut second = first.clone();
    second.input = json!({ "id": "tenant-2", "name": "Changed", "active": true });

    let created = runtime.invoke(first).unwrap();
    let repeated = runtime.invoke(second).unwrap();
    assert_eq!(created.output["id"], "tenant-1");
    assert_eq!(repeated.output["id"], "tenant-1");
    assert_eq!(repeated.output["data"]["name"], "Acme");
}
