use greentic_sorx_core::{
    BusinessEventSink, CommandEventInput, EntityEventInput, MemoryBusinessEventSink,
    command_event_envelope, entity_event_envelope, runtime_pack,
};
use serde_json::json;

#[test]
fn entity_envelope_maps_created_record() {
    let pack = runtime_pack("landlord", "0.1.0");
    let envelope = entity_event_envelope(
        &pack,
        EntityEventInput {
            environment: "local",
            tenant_id: "tenant-a",
            entity: "Tenant",
            operation: "created",
            record_id: "rec-1",
            record: Some(json!({"id": "rec-1", "name": "Acme"})),
            idempotency_key: Some("idem-1"),
        },
    )
    .unwrap();

    assert_eq!(envelope.topic, "sorla.landlord.Tenant.created");
    assert_eq!(envelope.r#type, "com.greentic.sorla.entity.created.v1");
    assert_eq!(envelope.source, "sorx:landlord:0.1.0");
    assert_eq!(envelope.tenant.env.as_str(), "local");
    assert_eq!(envelope.tenant.tenant.as_str(), "tenant-a");
    assert_eq!(envelope.subject.as_deref(), Some("Tenant:rec-1"));
    assert_eq!(envelope.correlation_id.as_deref(), Some("idem-1"));
    assert_eq!(
        envelope.metadata.get("idempotency_key").map(String::as_str),
        Some("idem-1")
    );
    assert_eq!(envelope.payload["entity"], "Tenant");
    assert_eq!(envelope.payload["id"], "rec-1");
    assert_eq!(envelope.payload["operation"], "created");
    assert_eq!(envelope.payload["record"]["name"], "Acme");
}

#[test]
fn entity_envelope_for_deleted_omits_record() {
    let pack = runtime_pack("landlord", "0.1.0");
    let envelope = entity_event_envelope(
        &pack,
        EntityEventInput {
            environment: "local",
            tenant_id: "tenant-a",
            entity: "Tenant",
            operation: "deleted",
            record_id: "rec-1",
            record: None,
            idempotency_key: None,
        },
    )
    .unwrap();

    assert_eq!(envelope.topic, "sorla.landlord.Tenant.deleted");
    assert!(envelope.payload.get("record").is_none());
    assert!(envelope.correlation_id.is_none());
}

#[test]
fn command_envelope_maps_emit_event_step() {
    let pack = runtime_pack("landlord", "0.1.0");
    let envelope = command_event_envelope(
        &pack,
        CommandEventInput {
            environment: "local",
            tenant_id: "tenant-a",
            event_name: "RecordRemoved",
            subject_entity: "Tenant",
            subject_id: "rec-9",
            payload: json!({"reason": "gdpr"}),
            idempotency_key: None,
        },
    )
    .unwrap();

    assert_eq!(envelope.topic, "sorla.landlord.RecordRemoved");
    assert_eq!(envelope.r#type, "com.greentic.sorla.RecordRemoved.v1");
    assert_eq!(envelope.subject.as_deref(), Some("Tenant:rec-9"));
    assert_eq!(envelope.payload["reason"], "gdpr");
}

#[test]
fn envelope_ids_are_unique_and_valid() {
    let pack = runtime_pack("landlord", "0.1.0");
    let first = entity_event_envelope(
        &pack,
        EntityEventInput {
            environment: "local",
            tenant_id: "t",
            entity: "E",
            operation: "created",
            record_id: "1",
            record: None,
            idempotency_key: None,
        },
    )
    .unwrap();
    let second = entity_event_envelope(
        &pack,
        EntityEventInput {
            environment: "local",
            tenant_id: "t",
            entity: "E",
            operation: "created",
            record_id: "1",
            record: None,
            idempotency_key: None,
        },
    )
    .unwrap();
    assert_ne!(first.id, second.id);
}

#[test]
fn invalid_tenant_identifier_is_an_error() {
    let pack = runtime_pack("landlord", "0.1.0");
    let result = entity_event_envelope(
        &pack,
        EntityEventInput {
            environment: "local",
            tenant_id: "bad tenant!",
            entity: "E",
            operation: "created",
            record_id: "1",
            record: None,
            idempotency_key: None,
        },
    );
    assert!(result.is_err());
}

#[test]
fn memory_sink_records_published_envelopes() {
    let pack = runtime_pack("landlord", "0.1.0");
    let sink = MemoryBusinessEventSink::new();
    let envelope = entity_event_envelope(
        &pack,
        EntityEventInput {
            environment: "local",
            tenant_id: "t",
            entity: "E",
            operation: "created",
            record_id: "1",
            record: None,
            idempotency_key: None,
        },
    )
    .unwrap();
    sink.publish(envelope.clone()).unwrap();
    let events = sink.events().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].topic, envelope.topic);
}
