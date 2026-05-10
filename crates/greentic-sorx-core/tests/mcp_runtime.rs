use std::sync::Arc;

use greentic_sorx_core::{
    CallerContext, EndpointRouter, EndpointStatus, McpRuntime, MemoryAuditSink,
    MemoryStoreProvider, ProviderRegistry, SorxRuntime, default_start_schema,
    mcp_tools_from_metadata, normalize_start_answers, runtime_config_from_answers, runtime_pack,
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
                "endpoint_id": "tenant.terminate",
                "operation_id": "tenant.terminate",
                "operation": "delete",
                "method": "DELETE",
                "path": "/v1/tenants/{id}",
                "entity": "Tenant",
                "collection": "tenants",
                "provider_binding": "store",
                "risk": "high"
            }
        ]
    })
}

fn tools() -> Value {
    json!({
        "schema": "greentic.sorla.mcp-tools.v1",
        "tools": [
            {
                "name": "sorla_create_tenant",
                "description": "Create a tenant record",
                "endpoint_id": "tenant.create",
                "input_schema": {}
            },
            {
                "name": "sorla_terminate_tenant",
                "endpoint_id": "tenant.terminate"
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

fn runtime(audit: Option<MemoryAuditSink>) -> (McpRuntime, Option<MemoryAuditSink>) {
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("landlord", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let tool_list = mcp_tools_from_metadata(Some(&tools()), &router).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_store("store", Arc::new(MemoryStoreProvider::new()));
    let mut sorx = SorxRuntime::new(runtime_pack("landlord", "0.1.0"), config, router, providers);
    if let Some(audit) = &audit {
        sorx = sorx.with_audit_sink(Arc::new(audit.clone()));
    }
    (McpRuntime::new(sorx, tool_list), audit)
}

fn caller() -> CallerContext {
    CallerContext {
        subject: "mcp.local".to_string(),
        roles: vec!["agent".to_string()],
    }
}

#[test]
fn mcp_tools_load_from_metadata_stably() {
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let list = mcp_tools_from_metadata(Some(&tools()), &router).unwrap();
    assert_eq!(list.schema, "greentic.sorx.mcp-tools.v1");
    assert_eq!(list.tools[0].name, "sorla_create_tenant");
    assert_eq!(list.tools[0].endpoint_id, "tenant.create");
    assert_eq!(list.tools[0].operation_id, "tenant.create");
    assert_eq!(
        format!("{:?}", list.tools[0].risk).to_ascii_lowercase(),
        "low"
    );
}

#[test]
fn mcp_create_tenant_uses_same_runtime_router() {
    let (runtime, _) = runtime(None);
    let result = runtime
        .call_tool(
            "sorla_create_tenant",
            "tenant-a",
            caller(),
            json!({ "id": "tenant-1", "name": "Acme", "active": true }),
            None,
        )
        .unwrap();
    assert_eq!(result.status, EndpointStatus::Created);
    assert_eq!(result.output["id"], "tenant-1");
}

#[test]
fn high_risk_mcp_tool_requires_approval() {
    let (runtime, _) = runtime(None);
    let result = runtime
        .call_tool(
            "sorla_terminate_tenant",
            "tenant-a",
            caller(),
            json!({ "id": "tenant-1" }),
            None,
        )
        .unwrap();
    assert_eq!(result.status, EndpointStatus::ApprovalRequired);
    assert_eq!(result.output["approval"]["risk"], "high");
}

#[test]
fn audit_records_mcp_source() {
    let audit = MemoryAuditSink::new();
    let (runtime, audit) = runtime(Some(audit));
    runtime
        .call_tool(
            "sorla_create_tenant",
            "tenant-a",
            caller(),
            json!({ "id": "tenant-1", "name": "Acme", "active": true }),
            Some("mcp-create".to_string()),
        )
        .unwrap();
    let events = audit.unwrap().events().unwrap();
    assert_eq!(events[0].details["source"], "mcp");
    assert!(events[0].idempotency_key_present);
}

#[test]
fn mcp_and_direct_runtime_produce_equivalent_results() {
    let (runtime, _) = runtime(None);
    let result = runtime
        .call_tool(
            "sorla_create_tenant",
            "tenant-a",
            caller(),
            json!({ "id": "tenant-1", "name": "Acme", "active": true }),
            None,
        )
        .unwrap();
    assert_eq!(result.output["data"]["name"], "Acme");
    assert_eq!(result.events[0].endpoint_id, "tenant.create");
}
