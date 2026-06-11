use std::sync::Arc;

use greentic_sorx_core::{
    BusinessEventSink, EndpointRouter, EndpointStatus, MemoryAuditSink, MemoryBusinessEventSink,
    MemoryStoreProvider, ProviderRegistry, SorxError, SorxResult, SorxRuntime,
    default_start_schema, invocation, normalize_start_answers, runtime_config_from_answers,
    runtime_pack,
};
use greentic_types::EventEnvelope;
use serde_json::json;

/// Sink that always fails publication; used to verify that a broken sink
/// never causes a business operation to fail.
struct FailingBusinessEventSink;

impl BusinessEventSink for FailingBusinessEventSink {
    fn publish(&self, _envelope: EventEnvelope) -> SorxResult<()> {
        Err(SorxError::new("sink_down", "boom"))
    }
}

fn gateway() -> serde_json::Value {
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
                "endpoint_id": "tenant.delete",
                "operation_id": "tenant.delete",
                "operation": "delete",
                "method": "DELETE",
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
                "endpoint_id": "tenant.query",
                "operation_id": "tenant.query",
                "operation": "query",
                "method": "POST",
                "path": "/v1/tenants/query",
                "entity": "Tenant",
                "collection": "tenants",
                "provider_binding": "store",
                "risk": "low"
            }
        ]
    })
}

fn answers() -> serde_json::Value {
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

/// Builds the base runtime without any overrides applied.
fn build_runtime() -> SorxRuntime {
    let normalized = normalize_start_answers(&default_start_schema(), &answers(), true).unwrap();
    let config = runtime_config_from_answers("landlord", &normalized.answers).unwrap();
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_canonical_store("store", Arc::new(MemoryStoreProvider::new()));
    SorxRuntime::new(runtime_pack("landlord", "0.1.0"), config, router, providers)
}

fn runtime_with_sink(sink: Arc<MemoryBusinessEventSink>) -> SorxRuntime {
    build_runtime().with_event_sink(sink)
}

fn runtime_default() -> SorxRuntime {
    build_runtime()
}

#[test]
fn create_update_delete_publish_lifecycle_events() {
    let sink = Arc::new(MemoryBusinessEventSink::new());
    let runtime = runtime_with_sink(sink.clone());

    // Create a tenant record
    let created = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "rec-1", "name": "Acme", "active": true }),
        ))
        .unwrap();
    assert_eq!(created.status, EndpointStatus::Created);

    // Update the record
    let updated = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.update",
            "tenant.update",
            json!({ "id": "rec-1", "patch": { "name": "Acme Corp" } }),
        ))
        .unwrap();
    assert_eq!(updated.status, EndpointStatus::Ok);

    // Delete the record
    let deleted = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.delete",
            "tenant.delete",
            json!({ "id": "rec-1" }),
        ))
        .unwrap();
    assert_eq!(deleted.status, EndpointStatus::Deleted);

    let events = sink.events().unwrap();
    assert_eq!(events.len(), 3, "expected exactly 3 business events");

    // Assert topics in order
    let topics: Vec<&str> = events.iter().map(|event| event.topic.as_str()).collect();
    assert_eq!(
        topics,
        vec![
            "sorla.landlord.Tenant.created",
            "sorla.landlord.Tenant.updated",
            "sorla.landlord.Tenant.deleted",
        ]
    );

    // Assert created event payload has record.data.name == "Acme" and subject Tenant:rec-1
    // The record snapshot is the serialized EntityRecord: { "id": "...", "data": {...}, "version": N }
    let created_event = &events[0];
    assert_eq!(
        created_event.payload["record"]["data"]["name"], "Acme",
        "created event must embed the record snapshot with data.name=Acme"
    );
    assert_eq!(
        created_event.subject.as_deref(),
        Some("Tenant:rec-1"),
        "created event subject must be Tenant:rec-1"
    );

    // Assert deleted event has no record key in payload
    let deleted_event = &events[2];
    assert!(
        deleted_event.payload.get("record").is_none(),
        "deleted event must NOT include a record snapshot"
    );

    // Assert all events have the correct tenant and environment
    for event in &events {
        assert_eq!(
            event.tenant.tenant_id.to_string(),
            "tenant-a",
            "all events must carry tenant-a"
        );
        assert_eq!(
            event.tenant.env.to_string(),
            "local",
            "all events must carry the local environment"
        );
    }
}

#[test]
fn get_and_query_do_not_publish() {
    let sink = Arc::new(MemoryBusinessEventSink::new());
    let runtime = runtime_with_sink(sink.clone());

    // Create first to have a record
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "rec-noread", "name": "GetTest", "active": true }),
        ))
        .unwrap();

    // Get — must NOT emit a business event
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.get",
            "tenant.get",
            json!({ "id": "rec-noread" }),
        ))
        .unwrap();

    // Query — must NOT emit a business event
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.query",
            "tenant.query",
            json!({ "filter": {} }),
        ))
        .unwrap();

    let events = sink.events().unwrap();
    assert_eq!(
        events.len(),
        1,
        "only the create should have published a business event; get and query must not"
    );
    assert_eq!(events[0].topic, "sorla.landlord.Tenant.created");
}

#[test]
fn delete_of_missing_record_publishes_nothing() {
    let sink = Arc::new(MemoryBusinessEventSink::new());
    let runtime = runtime_with_sink(sink.clone());

    // Delete a record that does not exist — should return NotFound and emit nothing
    let result = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.delete",
            "tenant.delete",
            json!({ "id": "does-not-exist" }),
        ))
        .unwrap();
    assert_eq!(result.status, EndpointStatus::NotFound);

    let events = sink.events().unwrap();
    assert!(
        events.is_empty(),
        "deleting a missing record must not publish a business event"
    );
}

#[test]
fn default_runtime_keeps_disabled_sink_and_behavior() {
    let runtime = runtime_default();

    // Create should still succeed with the disabled (no-op) sink
    let created = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "rec-default", "name": "Default", "active": true }),
        ))
        .unwrap();
    assert_eq!(
        created.status,
        EndpointStatus::Created,
        "create must succeed even without a configured event sink"
    );
}

#[test]
fn failing_sink_never_fails_operation_and_emits_audit() {
    let memory_audit = Arc::new(MemoryAuditSink::new());
    let runtime = build_runtime()
        .with_event_sink(Arc::new(FailingBusinessEventSink))
        .with_audit_sink(memory_audit.clone());

    // A create operation must succeed even though the sink always returns an error.
    let result = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "id": "rec-fail-sink", "name": "FailSink", "active": true }),
        ))
        .unwrap();
    assert_eq!(
        result.status,
        EndpointStatus::Created,
        "create must succeed even when the business-event sink always fails"
    );

    // The failed publish attempt must surface as an audit event.
    let audit_events = memory_audit.events().unwrap();
    let publish_failed_events: Vec<_> = audit_events
        .iter()
        .filter(|audit_event| audit_event.event == "sorx.business_event.publish_failed")
        .collect();
    assert_eq!(
        publish_failed_events.len(),
        1,
        "a failing sink must emit exactly one sorx.business_event.publish_failed audit event"
    );
}
