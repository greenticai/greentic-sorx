use std::sync::Arc;
use std::sync::Mutex;

use greentic_sorx_core::{
    BoundControlHook, CONTRACT_CONTROL_PRE_CALL_V1, ControlDecision, ControlHook, EndpointRouter,
    EndpointStatus, ExtensionFailMode, MemoryStoreProvider, ObserverEvent, ObserverHook,
    ProviderNamespace, ProviderRegistry, QueryOp, RuntimeExtensionAdapter, RuntimeExtensionBinding,
    RuntimeExtensionRegistry, RuntimeExtensions, RuntimeOperationalIndex, SorStoreProvider,
    SorxError, SorxResult, SorxRuntime, StackCallContext, StackCallRequest, StackCallResponse,
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

#[derive(Debug)]
struct DenyControl;

impl ControlHook for DenyControl {
    fn pre_call(
        &self,
        _context: &StackCallContext,
        request: &StackCallRequest,
    ) -> SorxResult<ControlDecision> {
        if request.operation_id == "tenant.create" {
            Ok(ControlDecision::deny("blocked by control"))
        } else {
            Ok(ControlDecision::allow())
        }
    }
}

#[derive(Debug)]
struct PatchControl {
    pre_patch: Option<Value>,
    post_patch: Option<Value>,
}

impl ControlHook for PatchControl {
    fn pre_call(
        &self,
        _context: &StackCallContext,
        _request: &StackCallRequest,
    ) -> SorxResult<ControlDecision> {
        Ok(self
            .pre_patch
            .clone()
            .map(ControlDecision::allow_with_patch)
            .unwrap_or_else(ControlDecision::allow))
    }

    fn post_call(
        &self,
        _context: &StackCallContext,
        _request: &StackCallRequest,
        _response: &StackCallResponse,
    ) -> SorxResult<ControlDecision> {
        Ok(self
            .post_patch
            .clone()
            .map(ControlDecision::allow_with_patch)
            .unwrap_or_else(ControlDecision::allow))
    }
}

#[derive(Debug, Default)]
struct RecordingObserver {
    events: Mutex<Vec<String>>,
    payloads: Mutex<Vec<ObserverEvent>>,
}

impl RecordingObserver {
    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    fn payloads(&self) -> Vec<ObserverEvent> {
        self.payloads.lock().unwrap().clone()
    }
}

impl ObserverHook for RecordingObserver {
    fn observe(&self, event: &ObserverEvent) -> SorxResult<()> {
        self.events.lock().unwrap().push(event.event_type.clone());
        self.payloads.lock().unwrap().push(event.clone());
        Ok(())
    }
}

#[derive(Debug)]
struct FailingObserver;

impl ObserverHook for FailingObserver {
    fn observe(&self, _event: &ObserverEvent) -> SorxResult<()> {
        Err(SorxError::new("observer_failed", "observer failed"))
    }
}

#[derive(Debug)]
struct BoundDenyExtension;

impl RuntimeExtensionAdapter for BoundDenyExtension {
    fn control(
        &self,
        _hook: &str,
        _binding: &RuntimeExtensionBinding,
        request: &Value,
        _response: Option<&Value>,
    ) -> SorxResult<ControlDecision> {
        if request["operation_id"] == "tenant.create" {
            Ok(ControlDecision::deny("blocked by bound extension"))
        } else {
            Ok(ControlDecision::allow())
        }
    }
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

fn runtime_with_gateway_and_indexes(
    gateway: Value,
    indexes: Vec<RuntimeOperationalIndex>,
    provider: Arc<MemoryStoreProvider>,
) -> SorxRuntime {
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("landlord", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&gateway).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_canonical_store("store", provider);
    let mut pack = runtime_pack("landlord", "0.1.0");
    pack.operational_indexes = indexes;
    SorxRuntime::new(pack, config, router, providers)
}

fn unique_index(id: &str, record: &str, fields: &[&str]) -> RuntimeOperationalIndex {
    RuntimeOperationalIndex {
        id: id.to_string(),
        record: record.to_string(),
        collection: None,
        kind: if fields.len() == 1 {
            "exact"
        } else {
            "composite"
        }
        .to_string(),
        fields: fields.iter().map(|field| (*field).to_string()).collect(),
        unique: true,
    }
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
fn control_pre_deny_prevents_provider_call() {
    let runtime = runtime().with_control_hook(Arc::new(DenyControl));
    let denied = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-denied", "name": "Acme", "active": true }),
        ))
        .unwrap();
    assert_eq!(denied.status, EndpointStatus::Denied);
    assert_eq!(denied.output["reason"], "blocked by control");

    let missing = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.get",
            "tenant.get",
            json!({ "id": "tenant-denied" }),
        ))
        .unwrap();
    assert_eq!(missing.status, EndpointStatus::NotFound);
}

#[test]
fn bound_control_extension_deny_prevents_provider_call() {
    let mut extensions = RuntimeExtensions::default();
    extensions.control.hooks.insert(
        "pre_call".to_string(),
        vec![RuntimeExtensionBinding {
            id: "policy".to_string(),
            contract: CONTRACT_CONTROL_PRE_CALL_V1.to_string(),
            pack_ref: "extensions/policy.gtpack".to_string(),
            fail_mode: ExtensionFailMode::Closed,
        }],
    );
    let control = BoundControlHook::new(
        extensions,
        RuntimeExtensionRegistry::new()
            .with_adapter("extensions/policy.gtpack", Arc::new(BoundDenyExtension)),
    );
    let runtime = runtime().with_control_hook(Arc::new(control));
    let denied = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-bound-denied", "name": "Acme", "active": true }),
        ))
        .unwrap();
    assert_eq!(denied.status, EndpointStatus::Denied);
    assert_eq!(denied.output["reason"], "blocked by bound extension");

    let missing = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.get",
            "tenant.get",
            json!({ "id": "tenant-bound-denied" }),
        ))
        .unwrap();
    assert_eq!(missing.status, EndpointStatus::NotFound);
}

#[test]
fn control_pre_patch_mutates_request_before_stack_invoke() {
    let runtime = runtime().with_control_hook(Arc::new(PatchControl {
        pre_patch: Some(json!({ "name": "Patched" })),
        post_patch: None,
    }));
    let created = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-patched", "name": "Original", "active": true }),
        ))
        .unwrap();
    assert_eq!(created.status, EndpointStatus::Created);
    assert_eq!(created.output["data"]["name"], "Patched");
}

#[test]
fn control_post_patch_mutates_response_after_stack_invoke() {
    let runtime = runtime().with_control_hook(Arc::new(PatchControl {
        pre_patch: None,
        post_patch: Some(json!({ "data": { "name": "Response Patched" } })),
    }));
    let created = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-post-patched", "name": "Original", "active": true }),
        ))
        .unwrap();
    assert_eq!(created.status, EndpointStatus::Created);
    assert_eq!(created.output["data"]["name"], "Response Patched");
}

#[test]
fn observer_receives_pre_and_post_events() {
    let observer = Arc::new(RecordingObserver::default());
    let runtime = runtime().with_observer_hook(observer.clone(), true);
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-observed", "name": "Acme", "active": true }),
        ))
        .unwrap();
    assert_eq!(
        observer.events(),
        vec!["stack.call.started", "stack.call.completed"]
    );
    let payloads = observer.payloads();
    assert_eq!(payloads[0].context.environment_id, "local");
    assert_eq!(payloads[0].context.runtime_id, "runtime-main");
    assert_eq!(payloads[0].context.deployment_id, "landlord");
    assert_eq!(payloads[0].context.route_id, "tenant.create");
    assert!(payloads[0].context.call_id.starts_with("call-"));
    assert_eq!(payloads[0].context.call_id, payloads[1].context.trace_id);
}

#[test]
fn observer_failure_fail_open_continues_stack_invoke() {
    let runtime = runtime().with_observer_hook(Arc::new(FailingObserver), true);
    let created = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-observer-fail-open", "name": "Acme", "active": true }),
        ))
        .unwrap();
    assert_eq!(created.status, EndpointStatus::Created);
}

#[test]
fn observer_failure_fail_closed_blocks_stack_invoke() {
    let runtime = runtime().with_observer_hook(Arc::new(FailingObserver), false);
    let err = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-observer-fail-closed", "name": "Acme", "active": true }),
        ))
        .unwrap_err();
    assert_eq!(err.code, "observer_failed");
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
fn runtime_create_rejects_duplicate_single_field_unique_index() {
    let runtime = runtime_with_gateway_and_indexes(
        gateway(),
        vec![unique_index("tenant_name_unique", "Tenant", &["name"])],
        Arc::new(MemoryStoreProvider::new()),
    );
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-1", "name": "Acme", "active": true }),
        ))
        .unwrap();

    let err = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-2", "name": "Acme", "active": true }),
        ))
        .unwrap_err();
    assert_eq!(err.code, "unique_constraint_violation");
}

#[test]
fn runtime_create_rejects_duplicate_composite_unique_index() {
    let runtime = runtime_with_gateway_and_indexes(
        gateway(),
        vec![unique_index(
            "tenant_name_active_unique",
            "Tenant",
            &["name", "active"],
        )],
        Arc::new(MemoryStoreProvider::new()),
    );
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-1", "name": "Acme", "active": true }),
        ))
        .unwrap();

    let err = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "tenant-2", "name": "Acme", "active": true }),
        ))
        .unwrap_err();
    assert_eq!(err.code, "unique_constraint_violation");
}

fn waiting_list_gateway() -> Value {
    json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            {
                "endpoint_id": "join_waiting_list",
                "operation_id": "join_waiting_list",
                "operation": "command",
                "method": "POST",
                "path": "/v1/agent/waiting_list_entries/join_waiting_list",
                "entity": "waiting_list_entry",
                "collection": "waiting_list_entries",
                "provider_binding": "store",
                "risk": "medium",
                "command": {
                    "kind": "record_mutation",
                    "action": "join_waiting_list",
                    "target": "waiting_list_entries",
                    "idempotency": "return_existing",
                    "constraints": {
                        "idempotency": {
                            "fields": ["lab_id", "user_id"],
                            "index": "waiting_list_entry_lab_user_unique",
                            "mode": "return_existing"
                        },
                        "unique": [
                            {
                                "fields": ["lab_id", "invitation_code"],
                                "index": "waiting_list_entry_lab_invitation_code_unique",
                                "kind": "composite",
                                "record": "waiting_list_entry"
                            },
                            {
                                "fields": ["lab_id", "user_id"],
                                "index": "waiting_list_entry_lab_user_unique",
                                "kind": "composite",
                                "record": "waiting_list_entry"
                            }
                        ]
                    },
                    "steps": [
                        {
                            "op": "create",
                            "as": "entry",
                            "entity": "waiting_list_entry",
                            "collection": "waiting_list_entries",
                            "input": {
                                "entry_id": "$generated.uuid",
                                "lab_id": "$input.lab_id",
                                "user_id": "$input.user_id",
                                "email": "$input.email",
                                "name": "$input.name",
                                "invitation_code": "$generated.invitation_code",
                                "referral_code": "$generated.referral_code",
                                "invited_by_code": "$input.invited_by_code",
                                "referred_count": 0,
                                "joined_at": "$now"
                            }
                        },
                        {
                            "op": "increment_where",
                            "as": "referrer",
                            "entity": "waiting_list_entry",
                            "collection": "waiting_list_entries",
                            "where": {
                                "lab_id": "$input.lab_id",
                                "invitation_code": "$input.invited_by_code"
                            },
                            "increments": {
                                "referred_count": 1
                            },
                            "when": { "present": "$input.invited_by_code" }
                        },
                        {
                            "op": "query",
                            "as": "waiting_list",
                            "entity": "waiting_list_entry",
                            "collection": "waiting_list_entries",
                            "where": { "lab_id": "$input.lab_id" },
                            "order_by": [
                                { "field": "referred_count", "direction": "desc" },
                                { "field": "joined_at", "direction": "asc" }
                            ]
                        }
                    ],
                    "return": {
                        "entry_id": "$steps.entry.record.data.entry_id",
                        "invitation_code": "$steps.entry.record.data.invitation_code",
                        "referral_code": "$steps.entry.record.data.referral_code",
                        "number_in_waiting_list": "$steps.waiting_list.count",
                        "position": {
                            "rank": {
                                "records": "$steps.waiting_list.records",
                                "where": {
                                    "data.entry_id": "$steps.entry.record.data.entry_id"
                                }
                            }
                        }
                    }
                }
            },
            {
                "endpoint_id": "show_waiting_list",
                "operation_id": "show_waiting_list",
                "operation": "query",
                "method": "POST",
                "path": "/v1/agent/waiting_list_entries/query/show_waiting_list",
                "entity": "waiting_list_entry",
                "collection": "waiting_list_entries",
                "provider_binding": "store"
            }
        ]
    })
}

fn waiting_list_indexes() -> Vec<RuntimeOperationalIndex> {
    vec![
        unique_index(
            "waiting_list_entry_lab_invitation_code_unique",
            "waiting_list_entry",
            &["lab_id", "invitation_code"],
        ),
        unique_index(
            "waiting_list_entry_lab_user_unique",
            "waiting_list_entry",
            &["lab_id", "user_id"],
        ),
    ]
}

fn join_input(user_id: &str) -> Value {
    json!({
        "lab_id": "example",
        "user_id": user_id,
        "email": format!("{user_id}@example.com"),
        "name": user_id
    })
}

fn join_input_with_invite(user_id: &str, invited_by_code: &str) -> Value {
    json!({
        "lab_id": "example",
        "user_id": user_id,
        "email": format!("{user_id}@example.com"),
        "name": user_id,
        "invited_by_code": invited_by_code
    })
}

fn waiting_list_leave_gateway() -> Value {
    json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            {
                "endpoint_id": "waiting_list_entry.create",
                "operation_id": "waiting_list_entry.create",
                "operation": "create",
                "method": "POST",
                "path": "/v1/waiting-list-entries",
                "entity": "waiting_list_entry",
                "collection": "waiting_list_entries",
                "provider_binding": "store",
                "risk": "low"
            },
            {
                "endpoint_id": "leave_waiting_list",
                "operation_id": "leave_waiting_list",
                "operation": "command",
                "method": "POST",
                "path": "/v1/agent/waiting_list_entries/leave_waiting_list",
                "entity": "waiting_list_entry",
                "collection": "waiting_list_entries",
                "provider_binding": "store",
                "risk": "medium",
                "command": {
                    "kind": "record_mutation",
                    "action": "leave_waiting_list",
                    "target": "waiting_list_entries",
                    "steps": [
                        {
                            "op": "find_one",
                            "as": "entry",
                            "entity": "waiting_list_entry",
                            "collection": "waiting_list_entries",
                            "where": {
                                "lab_id": "$input.lab_id",
                                "user_id": "$input.user_id"
                            }
                        },
                        {
                            "op": "delete_where",
                            "as": "delete",
                            "entity": "waiting_list_entry",
                            "collection": "waiting_list_entries",
                            "where": {
                                "lab_id": "$input.lab_id",
                                "user_id": "$input.user_id"
                            }
                        },
                        {
                            "op": "increment_where",
                            "as": "referrer",
                            "entity": "waiting_list_entry",
                            "collection": "waiting_list_entries",
                            "where": {
                                "entry_id": "$steps.entry.data.referrer_entry_id"
                            },
                            "increments": {
                                "referred_count": -1
                            },
                            "when": {
                                "all": [
                                    { "present": "$steps.entry.data.referrer_entry_id" },
                                    { "eq": ["$steps.delete.deleted_count", 1] }
                                ]
                            }
                        }
                    ],
                    "return": {
                        "deleted_count": "$steps.delete.deleted_count",
                        "referrer_updated_count": "$steps.referrer.updated_count",
                        "referrer_skipped": "$steps.referrer.skipped"
                    }
                }
            }
        ]
    })
}

fn seed_waiting_list_record(runtime: &SorxRuntime, user_id: &str, fields: Value) {
    let mut record = serde_json::Map::new();
    record.insert("entry_id".to_string(), json!(user_id));
    record.insert("lab_id".to_string(), json!("example"));
    record.insert("user_id".to_string(), json!(user_id));
    record.insert("email".to_string(), json!(format!("{user_id}@example.com")));
    record.insert("name".to_string(), json!(user_id));
    record.insert(
        "invitation_code".to_string(),
        json!(format!("invite-{user_id}")),
    );
    record.insert(
        "referral_code".to_string(),
        json!(format!("refer-{user_id}")),
    );
    record.insert("referred_count".to_string(), json!(0));
    record.insert("joined_at".to_string(), json!("2026-05-27T00:00:00Z"));
    if let Some(fields) = fields.as_object() {
        for (key, value) in fields {
            record.insert(key.clone(), value.clone());
        }
    }
    runtime
        .invoke(invocation(
            "tenant-a",
            "waiting_list_entry.create",
            "waiting_list_entry.create",
            Value::Object(record),
        ))
        .unwrap();
}

fn waiting_list_records(provider: &MemoryStoreProvider) -> Vec<Value> {
    provider
        .query(QueryOp {
            namespace: ProviderNamespace {
                tenant_id: "tenant-a".to_string(),
                sor_name: "landlord".to_string(),
            },
            entity: "waiting_list_entry".to_string(),
            collection: "waiting_list_entries".to_string(),
            filter: json!({ "lab_id": "example" }),
            order_by: Vec::new(),
        })
        .unwrap()
        .records
        .into_iter()
        .map(|record| record.data)
        .collect()
}

#[test]
fn join_waiting_list_return_existing_creates_one_record() {
    let provider = Arc::new(MemoryStoreProvider::new());
    let runtime = runtime_with_gateway_and_indexes(
        waiting_list_gateway(),
        waiting_list_indexes(),
        provider.clone(),
    );

    let first = runtime
        .invoke(invocation(
            "tenant-a",
            "join_waiting_list",
            "join_waiting_list",
            join_input("user-1"),
        ))
        .unwrap();
    let second = runtime
        .invoke(invocation(
            "tenant-a",
            "join_waiting_list",
            "join_waiting_list",
            join_input("user-1"),
        ))
        .unwrap();

    assert_eq!(
        first.output["result"]["entry_id"],
        second.output["result"]["entry_id"]
    );
    let records = provider
        .query(QueryOp {
            namespace: ProviderNamespace {
                tenant_id: "tenant-a".to_string(),
                sor_name: "landlord".to_string(),
            },
            entity: "waiting_list_entry".to_string(),
            collection: "waiting_list_entries".to_string(),
            filter: json!({ "lab_id": "example" }),
            order_by: Vec::new(),
        })
        .unwrap();
    assert_eq!(records.records.len(), 1);
}

#[test]
fn join_waiting_list_generates_distinct_codes_for_distinct_users() {
    let runtime = runtime_with_gateway_and_indexes(
        waiting_list_gateway(),
        waiting_list_indexes(),
        Arc::new(MemoryStoreProvider::new()),
    );

    let first = runtime
        .invoke(invocation(
            "tenant-a",
            "join_waiting_list",
            "join_waiting_list",
            join_input("user-1"),
        ))
        .unwrap();
    let second = runtime
        .invoke(invocation(
            "tenant-a",
            "join_waiting_list",
            "join_waiting_list",
            join_input("user-2"),
        ))
        .unwrap();

    assert_ne!(
        first.output["result"]["invitation_code"],
        second.output["result"]["invitation_code"]
    );
    assert_ne!(
        first.output["result"]["referral_code"],
        second.output["result"]["referral_code"]
    );
}

#[test]
fn join_waiting_list_increments_referrer_and_returns_ordered_position() {
    let provider = Arc::new(MemoryStoreProvider::new());
    let runtime = runtime_with_gateway_and_indexes(
        waiting_list_gateway(),
        waiting_list_indexes(),
        provider.clone(),
    );

    let referrer = runtime
        .invoke(invocation(
            "tenant-a",
            "join_waiting_list",
            "join_waiting_list",
            join_input("referrer"),
        ))
        .unwrap();
    let invitation_code = referrer.output["result"]["invitation_code"]
        .as_str()
        .unwrap()
        .to_string();

    let referred = runtime
        .invoke(invocation(
            "tenant-a",
            "join_waiting_list",
            "join_waiting_list",
            join_input_with_invite("referred", &invitation_code),
        ))
        .unwrap();
    assert_eq!(referred.output["result"]["position"], 2);

    let records = provider
        .query(QueryOp {
            namespace: ProviderNamespace {
                tenant_id: "tenant-a".to_string(),
                sor_name: "landlord".to_string(),
            },
            entity: "waiting_list_entry".to_string(),
            collection: "waiting_list_entries".to_string(),
            filter: json!({ "lab_id": "example" }),
            order_by: vec![
                greentic_sorx_core::QueryOrder {
                    field: "referred_count".to_string(),
                    direction: greentic_sorx_core::QueryOrderDirection::Desc,
                },
                greentic_sorx_core::QueryOrder {
                    field: "joined_at".to_string(),
                    direction: greentic_sorx_core::QueryOrderDirection::Asc,
                },
            ],
        })
        .unwrap();
    assert_eq!(records.records[0].data["user_id"], "referrer");
    assert_eq!(records.records[0].data["referred_count"], 1);
    assert_eq!(records.records[1].data["user_id"], "referred");
    assert_eq!(records.records[1].data["referred_count"], 0);
}

#[test]
fn leave_waiting_list_decrements_referrer_once_with_negative_increment() {
    let provider = Arc::new(MemoryStoreProvider::new());
    let runtime = runtime_with_gateway_and_indexes(
        waiting_list_leave_gateway(),
        waiting_list_indexes(),
        provider.clone(),
    );
    seed_waiting_list_record(&runtime, "referrer", json!({ "referred_count": 1 }));
    seed_waiting_list_record(
        &runtime,
        "referred",
        json!({ "referrer_entry_id": "referrer" }),
    );

    let left = runtime
        .invoke(invocation(
            "tenant-a",
            "leave_waiting_list",
            "leave_waiting_list",
            json!({ "lab_id": "example", "user_id": "referred" }),
        ))
        .unwrap();
    assert_eq!(left.output["result"]["deleted_count"], 1);
    assert_eq!(left.output["result"]["referrer_updated_count"], 1);

    let records = waiting_list_records(&provider);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["user_id"], "referrer");
    assert_eq!(records[0]["referred_count"], 0);

    let repeated = runtime
        .invoke(invocation(
            "tenant-a",
            "leave_waiting_list",
            "leave_waiting_list",
            json!({ "lab_id": "example", "user_id": "referred" }),
        ))
        .unwrap();
    assert_eq!(repeated.output["result"]["deleted_count"], 0);
    assert_eq!(repeated.output["result"]["referrer_skipped"], true);

    let records = waiting_list_records(&provider);
    assert_eq!(records[0]["referred_count"], 0);
}

#[test]
fn leave_waiting_list_without_referrer_does_not_decrement_any_record() {
    let provider = Arc::new(MemoryStoreProvider::new());
    let runtime = runtime_with_gateway_and_indexes(
        waiting_list_leave_gateway(),
        waiting_list_indexes(),
        provider.clone(),
    );
    seed_waiting_list_record(&runtime, "other", json!({ "referred_count": 1 }));
    seed_waiting_list_record(&runtime, "direct", json!({}));

    let left = runtime
        .invoke(invocation(
            "tenant-a",
            "leave_waiting_list",
            "leave_waiting_list",
            json!({ "lab_id": "example", "user_id": "direct" }),
        ))
        .unwrap();
    assert_eq!(left.output["result"]["deleted_count"], 1);
    assert_eq!(left.output["result"]["referrer_skipped"], true);

    let records = waiting_list_records(&provider);
    let other = records
        .iter()
        .find(|record| record["user_id"] == "other")
        .unwrap();
    assert_eq!(other["referred_count"], 1);
}

#[test]
fn concurrent_join_waiting_list_creates_one_record_for_same_user() {
    let provider = Arc::new(MemoryStoreProvider::new());
    let runtime = Arc::new(runtime_with_gateway_and_indexes(
        waiting_list_gateway(),
        waiting_list_indexes(),
        provider.clone(),
    ));
    let mut handles = Vec::new();
    for _ in 0..12 {
        let runtime = runtime.clone();
        handles.push(std::thread::spawn(move || {
            runtime
                .invoke(invocation(
                    "tenant-a",
                    "join_waiting_list",
                    "join_waiting_list",
                    join_input("same-user"),
                ))
                .unwrap()
                .output["result"]["entry_id"]
                .clone()
        }));
    }
    let entry_ids = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert!(entry_ids.iter().all(|id| id == &entry_ids[0]));
    let records = provider
        .query(QueryOp {
            namespace: ProviderNamespace {
                tenant_id: "tenant-a".to_string(),
                sor_name: "landlord".to_string(),
            },
            entity: "waiting_list_entry".to_string(),
            collection: "waiting_list_entries".to_string(),
            filter: json!({ "lab_id": "example" }),
            order_by: Vec::new(),
        })
        .unwrap();
    assert_eq!(records.records.len(), 1);
}

#[test]
fn concurrent_join_waiting_list_generates_unique_codes_for_different_users() {
    let runtime = Arc::new(runtime_with_gateway_and_indexes(
        waiting_list_gateway(),
        waiting_list_indexes(),
        Arc::new(MemoryStoreProvider::new()),
    ));
    let mut handles = Vec::new();
    for index in 0..16 {
        let runtime = runtime.clone();
        handles.push(std::thread::spawn(move || {
            runtime
                .invoke(invocation(
                    "tenant-a",
                    "join_waiting_list",
                    "join_waiting_list",
                    join_input(&format!("user-{index}")),
                ))
                .unwrap()
                .output["result"]["invitation_code"]
                .clone()
        }));
    }
    let codes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    let unique = codes
        .iter()
        .map(|code| code.as_str().unwrap().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), codes.len());
}

#[test]
fn update_where_generated_values_are_fresh_and_unique() {
    let provider = Arc::new(MemoryStoreProvider::new());
    let runtime = runtime_with_gateway_and_indexes(
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
                    "risk": "low"
                },
                {
                    "endpoint_id": "record.assign_code",
                    "operation_id": "record.assign_code",
                    "operation": "command",
                    "method": "POST",
                    "path": "/v1/records/assign-code",
                    "entity": "Record",
                    "collection": "records",
                    "provider_binding": "store",
                    "risk": "low",
                    "command": {
                        "kind": "record_mutation",
                        "steps": [
                            {
                                "op": "update_where",
                                "as": "update",
                                "entity": "Record",
                                "collection": "records",
                                "where": { "id": "$input.id" },
                                "set": { "code": "$generated.short_code" }
                            }
                        ],
                        "return": {
                            "code": "$steps.update.records.0.data.code"
                        }
                    }
                }
            ]
        }),
        vec![unique_index("record_code_unique", "Record", &["code"])],
        provider.clone(),
    );
    runtime
        .invoke(invocation(
            "tenant-a",
            "record.create",
            "record.create",
            json!({ "id": "record-1", "name": "One" }),
        ))
        .unwrap();
    runtime
        .invoke(invocation(
            "tenant-a",
            "record.create",
            "record.create",
            json!({ "id": "record-2", "name": "Two" }),
        ))
        .unwrap();

    runtime
        .invoke(invocation(
            "tenant-a",
            "record.assign_code",
            "record.assign_code",
            json!({ "id": "record-1" }),
        ))
        .unwrap();
    runtime
        .invoke(invocation(
            "tenant-a",
            "record.assign_code",
            "record.assign_code",
            json!({ "id": "record-2" }),
        ))
        .unwrap();
    let records = provider
        .query(QueryOp {
            namespace: ProviderNamespace {
                tenant_id: "tenant-a".to_string(),
                sor_name: "landlord".to_string(),
            },
            entity: "Record".to_string(),
            collection: "records".to_string(),
            filter: json!({}),
            order_by: Vec::new(),
        })
        .unwrap();
    let codes = records
        .records
        .iter()
        .map(|record| record.data["code"].as_str().unwrap().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(codes.len(), 2);
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
