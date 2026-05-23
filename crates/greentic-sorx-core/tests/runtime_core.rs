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
    providers.register_canonical_store("store", Arc::new(MemoryStoreProvider::new()));
    SorxRuntime::new(runtime_pack("landlord", "0.1.0"), config, router, providers)
}

fn runtime_with_gateway(
    pack_version: &str,
    gateway: Value,
    provider: Arc<MemoryStoreProvider>,
) -> SorxRuntime {
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("landlord", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&gateway).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_canonical_store("store", provider);
    SorxRuntime::new(
        runtime_pack("landlord", pack_version),
        config,
        router,
        providers,
    )
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
fn command_delete_where_removes_matching_records() {
    let gateway = json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            {
                "endpoint_id": "record.create",
                "operation_id": "record.create",
                "operation": "create",
                "method": "POST",
                "path": "/v1/records",
                "entity": "Record",
                "collection": "records",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["id", "name"],
                    "properties": {
                        "id": { "type": "string" },
                        "name": { "type": "string" }
                    }
                }
            },
            {
                "endpoint_id": "record.get",
                "operation_id": "record.get",
                "operation": "get",
                "method": "GET",
                "path": "/v1/records/{id}",
                "entity": "Record",
                "collection": "records",
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
        "endpoint_id": "record.archive",
        "operation_id": "record.archive",
        "operation": "command",
        "method": "POST",
        "path": "/v1/records/archive",
        "entity": "Record",
        "collection": "records",
        "provider_binding": "store",
        "risk": "low",
        "input_schema": {
            "type": "object",
            "required": ["record_id"],
            "properties": {
                "record_id": { "type": "string" }
            }
        },
        "command": {
            "kind": "state_transition",
            "action": "archive_record",
            "idempotency": "required",
            "steps": [
                {
                    "op": "delete_where",
                    "where": {
                        "id": "$input.record_id"
                    }
                }
            ]
        }
            }
        ]
    });
    let provider = Arc::new(MemoryStoreProvider::new());
    let runtime = runtime_with_gateway("0.1.0", gateway, provider);
    runtime
        .invoke(invocation(
            "tenant-a",
            "record.create",
            "record.create",
            json!({ "id": "record-1", "name": "Example" }),
        ))
        .unwrap();

    let mut leave = invocation(
        "tenant-a",
        "record.archive",
        "record.archive",
        json!({ "record_id": "record-1" }),
    );
    let err = runtime.invoke(leave.clone()).unwrap_err();
    assert_eq!(err.code, "idempotency_key_required");

    leave.idempotency_key = Some("archive-record-1".to_string());
    let archived = runtime.invoke(leave).unwrap();
    assert_eq!(archived.status, EndpointStatus::Ok);
    assert_eq!(archived.output["action"], "archive_record");
    assert_eq!(archived.output["result"]["deleted_count"], 1);

    let fetched = runtime
        .invoke(invocation(
            "tenant-a",
            "record.get",
            "record.get",
            json!({ "id": "record-1" }),
        ))
        .unwrap();
    assert_eq!(fetched.status, EndpointStatus::NotFound);
}

#[test]
fn command_create_uses_resolved_input_values() {
    let gateway = json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            {
                "endpoint_id": "record.materialize",
                "operation_id": "record.materialize",
                "operation": "command",
                "method": "POST",
                "path": "/v1/records/materialize",
                "entity": "Record",
                "collection": "records",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["record_id", "name"],
                    "properties": {
                        "record_id": { "type": "string" },
                        "name": { "type": "string" }
                    }
                },
                "command": {
                    "kind": "state_transition",
                    "action": "materialize_record",
                    "steps": [
                        {
                            "op": "create",
                            "input": {
                                "id": "$input.record_id",
                                "name": "$input.name",
                                "active": true
                            }
                        }
                    ]
                }
            },
            {
                "endpoint_id": "record.get",
                "operation_id": "record.get",
                "operation": "get",
                "method": "GET",
                "path": "/v1/records/{id}",
                "entity": "Record",
                "collection": "records",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["id"],
                    "properties": {
                        "id": { "type": "string" }
                    }
                }
            }
        ]
    });
    let runtime = runtime_with_gateway("0.1.0", gateway, Arc::new(MemoryStoreProvider::new()));

    let created = runtime
        .invoke(invocation(
            "tenant-a",
            "record.materialize",
            "record.materialize",
            json!({ "record_id": "record-1", "name": "Example" }),
        ))
        .unwrap();
    assert_eq!(created.status, EndpointStatus::Ok);
    assert_eq!(created.output["result"]["created_count"], 1);

    let fetched = runtime
        .invoke(invocation(
            "tenant-a",
            "record.get",
            "record.get",
            json!({ "id": "record-1" }),
        ))
        .unwrap();
    assert_eq!(fetched.output["data"]["name"], "Example");
    assert_eq!(fetched.output["data"]["active"], true);
}

#[test]
fn command_update_where_supports_generated_values_step_refs_and_return_shape() {
    let gateway = json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            {
                "endpoint_id": "record.create",
                "operation_id": "record.create",
                "operation": "create",
                "method": "POST",
                "path": "/v1/records",
                "entity": "Record",
                "collection": "records",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["id", "name"],
                    "properties": {
                        "id": { "type": "string" },
                        "name": { "type": "string" }
                    }
                }
            },
            {
                "endpoint_id": "record.generate_code",
                "operation_id": "record.generate_code",
                "operation": "command",
                "method": "POST",
                "path": "/v1/records/generate-code",
                "entity": "Record",
                "collection": "records",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["record_id"],
                    "properties": {
                        "record_id": { "type": "string" }
                    }
                },
                "command": {
                    "kind": "record_mutation",
                    "action": "generate_code",
                    "steps": [
                        {
                            "op": "find_one",
                            "as": "record",
                            "where": { "id": "$input.record_id" },
                            "required": true
                        },
                        {
                            "op": "update_where",
                            "as": "update",
                            "where": { "id": "$input.record_id" },
                            "set": {
                                "code": {
                                    "coalesce": [
                                        "$steps.record.data.code",
                                        "$generated.short_code"
                                    ]
                                },
                                "updated_at": "$now"
                            }
                        }
                    ],
                    "return": {
                        "record_id": "$input.record_id",
                        "code": "$steps.update.records.0.data.code",
                        "updated_count": "$steps.update.updated_count"
                    }
                }
            },
            {
                "endpoint_id": "record.query",
                "operation_id": "record.query",
                "operation": "query",
                "method": "POST",
                "path": "/v1/records/query",
                "entity": "Record",
                "collection": "records",
                "provider_binding": "store",
                "risk": "low"
            }
        ]
    });
    let runtime = runtime_with_gateway("0.1.0", gateway, Arc::new(MemoryStoreProvider::new()));
    runtime
        .invoke(invocation(
            "tenant-a",
            "record.create",
            "record.create",
            json!({ "id": "record-1", "name": "Example" }),
        ))
        .unwrap();

    let first = runtime
        .invoke(invocation(
            "tenant-a",
            "record.generate_code",
            "record.generate_code",
            json!({ "record_id": "record-1" }),
        ))
        .unwrap();
    assert_eq!(first.status, EndpointStatus::Ok);
    assert_eq!(first.output["result"]["record_id"], "record-1");
    assert_eq!(first.output["result"]["updated_count"], 1);
    let code = first.output["result"]["code"].as_str().unwrap().to_string();
    assert!(!code.is_empty());

    let second = runtime
        .invoke(invocation(
            "tenant-a",
            "record.generate_code",
            "record.generate_code",
            json!({ "record_id": "record-1" }),
        ))
        .unwrap();
    assert_eq!(second.output["result"]["code"], code);

    let queried = runtime
        .invoke(invocation(
            "tenant-a",
            "record.query",
            "record.query",
            json!({ "filter": { "id": "record-1" } }),
        ))
        .unwrap();
    assert_eq!(queried.output["records"][0]["data"]["code"], code);
}

#[test]
fn command_foreach_runs_nested_steps_with_item_refs_and_rollups() {
    let gateway = json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            {
                "endpoint_id": "bulk.import",
                "operation_id": "bulk.import",
                "operation": "command",
                "method": "POST",
                "path": "/v1/bulk/import",
                "entity": "BulkImport",
                "collection": "bulk_imports",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["items", "performed_by"],
                    "properties": {
                        "items": { "type": "array" },
                        "performed_by": { "type": "string" }
                    }
                },
                "command": {
                    "kind": "bulk_mutation",
                    "action": "bulk_import",
                    "steps": [
                        {
                            "op": "foreach",
                            "as": "imported",
                            "items": "$input.items",
                            "do": [
                                {
                                    "op": "create",
                                    "as": "created",
                                    "entity": "$item.entity",
                                    "collection": "$item.collection",
                                    "input": "$item.data"
                                },
                                {
                                    "op": "create",
                                    "entity": "audit_record",
                                    "collection": "audit_records",
                                    "input": {
                                        "record_type": "$item.entity",
                                        "record_id": "$steps.created.record.id",
                                        "action_type": "bulk_import",
                                        "performed_by": "$input.performed_by",
                                        "performed_at": "$now"
                                    }
                                }
                            ]
                        }
                    ],
                    "return": {
                        "imported_count": "$steps.imported.count",
                        "created_count": "$steps.imported.created_count",
                        "records": "$steps.imported.records"
                    }
                }
            },
            {
                "endpoint_id": "entity_a.query",
                "operation_id": "entity_a.query",
                "operation": "query",
                "method": "POST",
                "path": "/v1/entity-as/query",
                "entity": "entity_a",
                "collection": "entity_as",
                "provider_binding": "store",
                "risk": "low"
            },
            {
                "endpoint_id": "audit.query",
                "operation_id": "audit.query",
                "operation": "query",
                "method": "POST",
                "path": "/v1/audit-records/query",
                "entity": "audit_record",
                "collection": "audit_records",
                "provider_binding": "store",
                "risk": "low"
            }
        ]
    });
    let runtime = runtime_with_gateway("0.1.0", gateway, Arc::new(MemoryStoreProvider::new()));

    let imported = runtime
        .invoke(invocation(
            "tenant-a",
            "bulk.import",
            "bulk.import",
            json!({
                "performed_by": "tester",
                "items": [
                    {
                        "entity": "entity_a",
                        "collection": "entity_as",
                        "data": {
                            "id": "a-1",
                            "a_string_attr": "Alpha"
                        }
                    }
                ]
            }),
        ))
        .unwrap();
    assert_eq!(imported.status, EndpointStatus::Ok);
    assert_eq!(imported.output["result"]["imported_count"], 1);
    assert_eq!(imported.output["result"]["created_count"], 2);
    assert_eq!(imported.output["result"]["records"][0]["id"], "a-1");

    let entity_query = runtime
        .invoke(invocation(
            "tenant-a",
            "entity_a.query",
            "entity_a.query",
            json!({ "filter": { "id": "a-1" } }),
        ))
        .unwrap();
    assert_eq!(
        entity_query.output["records"][0]["data"]["a_string_attr"],
        "Alpha"
    );

    let audit_query = runtime
        .invoke(invocation(
            "tenant-a",
            "audit.query",
            "audit.query",
            json!({ "filter": { "record_id": "a-1" } }),
        ))
        .unwrap();
    assert_eq!(
        audit_query.output["records"][0]["data"]["record_type"],
        "entity_a"
    );
    assert_eq!(
        audit_query.output["records"][0]["data"]["performed_by"],
        "tester"
    );
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

#[test]
fn versioned_views_share_canonical_state_with_field_mapping() {
    let provider = Arc::new(MemoryStoreProvider::new());
    let v1 = runtime_with_gateway(
        "1.1.0",
        json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "endpoints": [{
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "operation": "create",
                "method": "POST",
                "path": "/v1.1/tenants",
                "entity": "Tenant",
                "collection": "tenants",
                "provider_binding": "store",
                "risk": "low",
                "view": {
                    "view_to_canonical": { "fullName": "name" },
                    "canonical_to_view": { "name": "fullName" }
                }
            }]
        }),
        provider.clone(),
    );
    let v2 = runtime_with_gateway("2.0.0", gateway(), provider);

    v1.invoke(invocation(
        "tenant-a",
        "tenant.create",
        "tenant.create",
        json!({ "id": "tenant-1", "fullName": "Acme", "active": true }),
    ))
    .unwrap();

    let fetched = v2
        .invoke(invocation(
            "tenant-a",
            "tenant.get",
            "tenant.get",
            json!({ "id": "tenant-1" }),
        ))
        .unwrap();
    assert_eq!(fetched.output["data"]["name"], "Acme");
}

#[test]
fn read_only_view_rejects_mutations() {
    let runtime = runtime_with_gateway(
        "1.0.0",
        json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "endpoints": [{
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "operation": "create",
                "method": "POST",
                "path": "/v1.0/tenants",
                "entity": "Tenant",
                "collection": "tenants",
                "provider_binding": "store",
                "risk": "low",
                "view": { "read_only": true }
            }]
        }),
        Arc::new(MemoryStoreProvider::new()),
    );

    let err = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-1", "name": "Acme" }),
        ))
        .unwrap_err();
    assert_eq!(err.code, "view_read_only");
}

#[test]
fn query_endpoint_can_use_index_requirement() {
    let provider = Arc::new(MemoryStoreProvider::new());
    let runtime = runtime_with_gateway(
        "2.0.0",
        json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "endpoints": [
                {
                    "endpoint_id": "tenant.create",
                    "operation_id": "tenant.create",
                    "operation": "create",
                    "method": "POST",
                    "path": "/v2/tenants",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "risk": "low"
                },
                {
                    "endpoint_id": "tenant.by_property",
                    "operation_id": "tenant.by_property",
                    "operation": "query",
                    "method": "POST",
                    "path": "/v2/tenants/by-property",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "requires": {
                        "index": {
                            "name": "tenants_by_property",
                            "capability": "exact-index-query"
                        }
                    }
                }
            ]
        }),
        provider,
    );
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-1", "name": "Acme", "property_id": "property-1" }),
        ))
        .unwrap();
    let queried = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.by_property",
            "tenant.by_property",
            json!({ "filter": { "property_id": "property-1" } }),
        ))
        .unwrap();
    assert_eq!(queried.output["records"][0]["id"], "tenant-1");
}

#[test]
fn query_endpoint_can_use_traversal_requirement() {
    let provider = Arc::new(MemoryStoreProvider::new());
    let runtime = runtime_with_gateway(
        "2.0.0",
        json!({
            "schema": "greentic.sorla.agent-gateway.v1",
            "endpoints": [
                {
                    "endpoint_id": "tenant.create",
                    "operation_id": "tenant.create",
                    "operation": "create",
                    "method": "POST",
                    "path": "/v2/tenants",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "risk": "low"
                },
                {
                    "endpoint_id": "tenant.reachable",
                    "operation_id": "tenant.reachable",
                    "operation": "query",
                    "method": "POST",
                    "path": "/v2/tenants/reachable",
                    "entity": "Tenant",
                    "collection": "tenants",
                    "provider_binding": "store",
                    "requires": {
                        "traversal": {
                            "name": "tenant_graph",
                            "capability": "bounded-graph-traversal",
                            "max_depth": 2,
                            "relationships": ["tenant_has_lease"]
                        }
                    }
                }
            ]
        }),
        provider,
    );
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-1", "name": "Acme" }),
        ))
        .unwrap();
    let traversed = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.reachable",
            "tenant.reachable",
            json!({ "id": "tenant-1" }),
        ))
        .unwrap();
    assert_eq!(traversed.output["records"][0]["id"], "tenant-1");
}
