use std::sync::Arc;

use greentic_sorx_core::{
    CallerContext, EndpointRouter, EndpointStatus, McpRuntime, MemoryBusinessEventSink,
    MemoryStoreProvider, ProviderRegistry, SorxRuntime, default_start_schema, invocation,
    mcp_tools_from_metadata, normalize_start_answers, runtime_config_from_answers, runtime_pack,
};
use serde_json::{Value, json};

/// Builds a minimal gateway containing only a foreach command with an
/// `emit_event` step inside the loop body.  Used exclusively by the
/// `foreach_emit_event_publishes_once_per_item` test so that the shared
/// `gateway()` fixture is not modified.
fn foreach_emit_gateway() -> Value {
    json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            {
                "endpoint_id": "batch.notify",
                "operation_id": "batch.notify",
                "operation": "command",
                "method": "POST",
                "path": "/v1/batch/notify",
                "entity": "BatchNotify",
                "collection": "batch_notifies",
                "provider_binding": "store",
                "risk": "low",
                "input_schema": {
                    "type": "object",
                    "required": ["items"],
                    "properties": {
                        "items": { "type": "array" }
                    }
                },
                "command": {
                    "kind": "bulk_mutation",
                    "action": "batch_notify",
                    "steps": [
                        {
                            "op": "foreach",
                            "as": "notified",
                            "items": "$input.items",
                            "do": [
                                {
                                    "op": "emit_event",
                                    "as": "notification",
                                    "event": "item_notified",
                                    "payload": {
                                        "item_id": "$item.id"
                                    }
                                }
                            ]
                        }
                    ],
                    "return": {
                        "notified_count": "$steps.notified.count"
                    }
                }
            }
        ]
    })
}

fn gateway() -> Value {
    json!({
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
                        "name": { "type": "string" },
                        "active": { "type": "boolean" }
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
                "risk": "low"
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
                    "required": ["id"],
                    "properties": { "id": { "type": "string" } }
                },
                "command": {
                    "kind": "record_mutation",
                    "action": "generate_code",
                    "steps": [
                        {
                            "op": "find_one",
                            "as": "record",
                            "where": { "id": "$input.id" },
                            "required": true
                        },
                        {
                            "op": "update_where",
                            "as": "update",
                            "where": { "id": "$input.id" },
                            "set": {
                                "code": {
                                    "coalesce": [
                                        "$steps.record.data.code",
                                        "$generated.short_code"
                                    ]
                                },
                                "updated_at": "$now"
                            }
                        },
                        {
                            "op": "emit_event",
                            "as": "event",
                            "event": "record_code_generated",
                            "payload": {
                                "record_id": "$input.id",
                                "code": "$steps.update.records.0.data.code"
                            }
                        }
                    ],
                    "return": {
                        "id": "$input.id",
                        "code": "$steps.update.records.0.data.code",
                        "updated_count": "$steps.update.updated_count"
                    }
                }
            },
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
                                    "entity": "AuditRecord",
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
                        "records": "$steps.imported.records"
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
                "command": {
                    "kind": "record_mutation",
                    "action": "archive_record",
                    "steps": [
                        {
                            "op": "delete_where",
                            "as": "delete",
                            "where": { "id": "$input.id" }
                        }
                    ],
                    "return": {
                        "deleted_count": "$steps.delete.deleted_count"
                    }
                }
            }
        ]
    })
}

fn tools() -> Value {
    json!({
        "schema": "greentic.sorla.mcp-tools.v1",
        "tools": [
            { "name": "record_create", "endpoint_id": "record.create" },
            { "name": "record_generate_code", "endpoint_id": "record.generate_code" },
            { "name": "bulk_import", "endpoint_id": "bulk.import" }
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
        "deployment": { "tenant_id": "tenant-a", "sor_name": "command-e2e", "environment": "local" },
        "exposure": {},
        "ghcr": {}
    })
}

fn runtime() -> SorxRuntime {
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("command-e2e", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_canonical_store("store", Arc::new(MemoryStoreProvider::new()));
    SorxRuntime::new(
        runtime_pack("command-e2e", "0.1.0"),
        config,
        router,
        providers,
    )
}

fn mcp_runtime() -> McpRuntime {
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let tool_list = mcp_tools_from_metadata(Some(&tools()), &router).unwrap();
    McpRuntime::new(runtime(), tool_list)
}

fn caller() -> CallerContext {
    CallerContext {
        subject: "e2e".to_string(),
        roles: vec!["tester".to_string()],
    }
}

#[test]
fn command_e2e_covers_crud_commands_bulk_events_and_mcp() {
    let runtime = runtime();

    let created = runtime
        .invoke(invocation(
            "tenant-a",
            "record.create",
            "record.create",
            json!({ "id": "record-1", "name": "Alpha", "active": true }),
        ))
        .unwrap();
    assert_eq!(created.status, EndpointStatus::Created);

    let first_code = runtime
        .invoke(invocation(
            "tenant-a",
            "record.generate_code",
            "record.generate_code",
            json!({ "id": "record-1" }),
        ))
        .unwrap();
    assert_eq!(first_code.status, EndpointStatus::Ok);
    let code = first_code.output["result"]["code"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!code.is_empty());

    let second_code = runtime
        .invoke(invocation(
            "tenant-a",
            "record.generate_code",
            "record.generate_code",
            json!({ "id": "record-1" }),
        ))
        .unwrap();
    assert_eq!(second_code.output["result"]["code"], code);

    let bulk = runtime
        .invoke(invocation(
            "tenant-a",
            "bulk.import",
            "bulk.import",
            json!({
                "performed_by": "tester",
                "items": [
                    {
                        "entity": "Record",
                        "collection": "records",
                        "data": { "id": "record-2", "name": "Beta", "active": true }
                    },
                    {
                        "entity": "OtherRecord",
                        "collection": "other_records",
                        "data": { "id": "other-1", "name": "Other" }
                    }
                ]
            }),
        ))
        .unwrap();
    assert_eq!(bulk.output["result"]["imported_count"], 2);
    assert_eq!(
        bulk.output["result"]["records"].as_array().unwrap().len(),
        4
    );

    let records = runtime
        .invoke(invocation(
            "tenant-a",
            "record.query",
            "record.query",
            json!({ "filter": {} }),
        ))
        .unwrap();
    assert_eq!(records.output["records"].as_array().unwrap().len(), 2);

    let archived = runtime
        .invoke(invocation(
            "tenant-a",
            "record.archive",
            "record.archive",
            json!({ "id": "record-2" }),
        ))
        .unwrap();
    assert_eq!(archived.output["result"]["deleted_count"], 1);

    let mcp = mcp_runtime();
    mcp.call_tool(
        "record_create",
        "tenant-a",
        caller(),
        json!({ "id": "mcp-1", "name": "MCP", "active": true }),
        None,
    )
    .unwrap();
    let generated = mcp
        .call_tool(
            "record_generate_code",
            "tenant-a",
            caller(),
            json!({ "id": "mcp-1" }),
            None,
        )
        .unwrap();
    assert_eq!(generated.status, EndpointStatus::Ok);
    assert!(generated.output["result"]["code"].as_str().is_some());
}

#[test]
fn command_e2e_stress_repeated_mutations_remain_consistent() {
    let runtime = runtime();
    let total = 150usize;

    for index in 0..total {
        runtime
            .invoke(invocation(
                "tenant-a",
                "record.create",
                "record.create",
                json!({
                    "id": format!("stress-{index}"),
                    "name": format!("Stress {index}"),
                    "active": index % 2 == 0
                }),
            ))
            .unwrap();
    }

    for index in 0..total {
        let first = runtime
            .invoke(invocation(
                "tenant-a",
                "record.generate_code",
                "record.generate_code",
                json!({ "id": format!("stress-{index}") }),
            ))
            .unwrap();
        let second = runtime
            .invoke(invocation(
                "tenant-a",
                "record.generate_code",
                "record.generate_code",
                json!({ "id": format!("stress-{index}") }),
            ))
            .unwrap();
        assert_eq!(
            first.output["result"]["code"],
            second.output["result"]["code"]
        );
    }

    let imported = runtime
        .invoke(invocation(
            "tenant-a",
            "bulk.import",
            "bulk.import",
            json!({
                "performed_by": "stress",
                "items": (0..50)
                    .map(|index| json!({
                        "entity": "StressImported",
                        "collection": "stress_imported",
                        "data": {
                            "id": format!("imported-{index}"),
                            "name": format!("Imported {index}")
                        }
                    }))
                    .collect::<Vec<_>>()
            }),
        ))
        .unwrap();
    assert_eq!(imported.output["result"]["imported_count"], 50);

    for index in (0..total).step_by(3) {
        let archived = runtime
            .invoke(invocation(
                "tenant-a",
                "record.archive",
                "record.archive",
                json!({ "id": format!("stress-{index}") }),
            ))
            .unwrap();
        assert_eq!(archived.output["result"]["deleted_count"], 1);
    }

    let remaining = runtime
        .invoke(invocation(
            "tenant-a",
            "record.query",
            "record.query",
            json!({ "filter": {} }),
        ))
        .unwrap();
    assert_eq!(
        remaining.output["records"].as_array().unwrap().len(),
        total - total.div_ceil(3)
    );
}

#[test]
fn emit_event_step_publishes_business_event_via_sink() {
    let sink = Arc::new(MemoryBusinessEventSink::new());
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("command-e2e", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_canonical_store("store", Arc::new(MemoryStoreProvider::new()));
    let runtime = SorxRuntime::new(
        runtime_pack("command-e2e", "0.1.0"),
        config,
        router,
        providers,
    )
    .with_event_sink(sink.clone());

    // Create a record first so the generate_code command has something to update
    runtime
        .invoke(invocation(
            "tenant-a",
            "record.create",
            "record.create",
            json!({ "id": "evt-record-1", "name": "EventTest", "active": true }),
        ))
        .unwrap();

    // Run the command that contains an emit_event step
    let result = runtime
        .invoke(invocation(
            "tenant-a",
            "record.generate_code",
            "record.generate_code",
            json!({ "id": "evt-record-1" }),
        ))
        .unwrap();
    assert_eq!(result.status, EndpointStatus::Ok);

    let events = sink.events().unwrap();

    // The create publishes a `created` entity event; the emit_event step publishes
    // the command event.  Filter to find the command event by topic pattern.
    let command_events: Vec<_> = events
        .iter()
        .filter(|event| event.topic == "sorla.command-e2e.record_code_generated")
        .collect();
    assert_eq!(
        command_events.len(),
        1,
        "emit_event step must publish exactly one business event with the record_code_generated topic"
    );
    let command_event = command_events[0];
    assert!(
        command_event.r#type.ends_with(".v1"),
        "business event type must end with .v1, got: {}",
        command_event.r#type
    );
}

#[test]
fn foreach_emit_event_publishes_once_per_item() {
    let sink = Arc::new(MemoryBusinessEventSink::new());
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("command-e2e", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&foreach_emit_gateway()).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_canonical_store("store", Arc::new(MemoryStoreProvider::new()));
    let runtime = SorxRuntime::new(
        runtime_pack("command-e2e", "0.1.0"),
        config,
        router,
        providers,
    )
    .with_event_sink(sink.clone());

    let result = runtime
        .invoke(invocation(
            "tenant-a",
            "batch.notify",
            "batch.notify",
            json!({
                "items": [
                    { "id": "item-1" },
                    { "id": "item-2" }
                ]
            }),
        ))
        .unwrap();
    assert_eq!(result.status, EndpointStatus::Ok);

    let events = sink.events().unwrap();
    let item_notified_events: Vec<_> = events
        .iter()
        .filter(|event| event.topic == "sorla.command-e2e.item_notified")
        .collect();
    assert_eq!(
        item_notified_events.len(),
        2,
        "emit_event inside a foreach must publish exactly one event per iteration; \
         expected 2 item_notified events for 2 items, got {}",
        item_notified_events.len()
    );
}
