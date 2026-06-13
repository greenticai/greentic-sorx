//! Business event publication: maps Sorx record/command activity onto the
//! canonical Greentic event envelope and hands it to a pluggable sink.
//!
//! Mirrors the `audit` module pattern. Publication is best-effort: callers
//! must never fail a business operation because a sink failed.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use greentic_types::{EnvId, EventEnvelope, EventId, TenantCtx, TenantId};
use serde_json::{Value, json};

use crate::{RuntimePack, SorxError, SorxResult};

/// Sink that receives published business-event envelopes.
///
/// Implementations must be `Send + Sync` so they can be held behind an `Arc`
/// and shared across threads in the synchronous Sorx runtime.  Publication is
/// best-effort — callers should log and continue on error rather than
/// propagating a sink failure back to the originating business operation.
pub trait BusinessEventSink: Send + Sync {
    fn publish(&self, envelope: EventEnvelope) -> SorxResult<()>;
}

/// No-op sink that silently discards every envelope.
///
/// Use this when business-event publication is disabled in the runtime
/// configuration.
#[derive(Debug, Default)]
pub struct DisabledBusinessEventSink;

impl BusinessEventSink for DisabledBusinessEventSink {
    fn publish(&self, _envelope: EventEnvelope) -> SorxResult<()> {
        Ok(())
    }
}

/// Sink that serializes every envelope to JSON and prints it to stdout.
///
/// Intended for local development and smoke-testing.
#[derive(Debug, Default)]
pub struct StdoutBusinessEventSink;

impl BusinessEventSink for StdoutBusinessEventSink {
    fn publish(&self, envelope: EventEnvelope) -> SorxResult<()> {
        let encoded = serde_json::to_string(&envelope)
            .map_err(|err| SorxError::new("event_encode_failed", err.to_string()))?;
        println!("{encoded}");
        Ok(())
    }
}

/// In-memory sink that accumulates every published envelope.
///
/// Primarily used in unit tests to assert that the correct envelopes were
/// emitted without needing a real transport.
#[derive(Debug, Clone, Default)]
pub struct MemoryBusinessEventSink {
    events: Arc<Mutex<Vec<EventEnvelope>>>,
}

impl MemoryBusinessEventSink {
    /// Creates a new empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of all envelopes published so far.
    ///
    /// # Errors
    ///
    /// Returns an error if the internal mutex was poisoned by a panicking
    /// thread.
    pub fn events(&self) -> SorxResult<Vec<EventEnvelope>> {
        self.events
            .lock()
            .map_err(|_| SorxError::new("event_lock_failed", "event sink lock was poisoned"))
            .map(|guard| guard.clone())
    }
}

impl BusinessEventSink for MemoryBusinessEventSink {
    fn publish(&self, envelope: EventEnvelope) -> SorxResult<()> {
        self.events
            .lock()
            .map_err(|_| SorxError::new("event_lock_failed", "event sink lock was poisoned"))?
            .push(envelope);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Envelope builders
// ---------------------------------------------------------------------------

/// Input parameters for building a record-lifecycle event envelope.
///
/// Using a named struct prevents silently-swapped positional `&str` arguments
/// when the caller constructs the envelope.
#[derive(Debug, Clone)]
pub struct EntityEventInput<'a> {
    /// Environment identifier (for example `local`, `prod`).
    pub environment: &'a str,
    /// Tenant identifier string; must be a valid Greentic identifier.
    pub tenant_id: &'a str,
    /// Entity type name (for example `Tenant`).
    pub entity: &'a str,
    /// Operation label (for example `created`, `updated`, `deleted`).
    pub operation: &'a str,
    /// Stable identifier of the affected record.
    pub record_id: &'a str,
    /// Optional full record snapshot to embed in the payload.
    pub record: Option<Value>,
    /// Optional idempotency key forwarded as `correlation_id` and into `metadata`.
    pub idempotency_key: Option<&'a str>,
}

/// Input parameters for building a named command or domain event envelope.
///
/// Using a named struct prevents silently-swapped positional `&str` arguments
/// when the caller constructs the envelope.
#[derive(Debug, Clone)]
pub struct CommandEventInput<'a> {
    /// Environment identifier string.
    pub environment: &'a str,
    /// Tenant identifier string; must be a valid Greentic identifier.
    pub tenant_id: &'a str,
    /// Stable event name (for example `RecordRemoved`).
    pub event_name: &'a str,
    /// Entity type that the event concerns.
    pub subject_entity: &'a str,
    /// Record identifier that forms the `subject` field.
    pub subject_id: &'a str,
    /// Full event payload as a JSON value.
    pub payload: Value,
    /// Optional idempotency key forwarded as `correlation_id` and into `metadata`.
    pub idempotency_key: Option<&'a str>,
}

/// Per-process tag initialised once at first use.
///
/// Mixes the OS process id with the subsecond nanoseconds of the startup
/// instant so two replicas or restarts in the same millisecond cannot collide.
static PROCESS_TAG: OnceLock<String> = OnceLock::new();

fn process_tag() -> &'static str {
    PROCESS_TAG.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos())
            .unwrap_or_default();
        format!("{:x}-{:x}", std::process::id(), nanos)
    })
}

/// Monotonically increasing nonce used together with the millisecond timestamp
/// and the process tag to produce approximately time-ordered,
/// uniqueness-focused event identifiers.
static EVENT_NONCE: AtomicU64 = AtomicU64::new(1);

/// Generates the next unique event identifier.
fn next_event_id() -> SorxResult<EventId> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let nonce = EVENT_NONCE.fetch_add(1, Ordering::Relaxed);
    EventId::new(format!("sorla-{}-{millis}-{nonce}", process_tag()))
        .map_err(|err| SorxError::new("event_id_invalid", err.to_string()))
}

/// Sanitizes a value for use as a NATS subject segment.
///
/// Only ASCII alphanumeric characters, hyphens, and underscores are kept.
/// Any other character (including spaces and dots) is replaced with a hyphen.
/// An empty result falls back to `"unknown"`.
fn topic_segment(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

/// Returns the canonical NATS topic for an entity lifecycle event.
///
/// Follows the pattern `sorla.<pack>.<entity>.<operation>` where each
/// segment is sanitized by [`topic_segment`].  This is the same string
/// written into [`EventEnvelope::topic`] by [`entity_event_envelope`].
///
/// Exposing this as a public function lets the CLI capability-offer layer
/// advertise the exact topic without duplicating the sanitization logic.
pub fn entity_event_topic(pack_name: &str, entity: &str, operation: &str) -> String {
    format!(
        "sorla.{}.{}.{}",
        topic_segment(pack_name),
        topic_segment(entity),
        topic_segment(operation),
    )
}

/// Returns the canonical NATS topic for a command-emitted (domain) event.
///
/// Follows the pattern `sorla.<pack>.<event_name>` where each segment is
/// sanitized by [`topic_segment`].  This is the same string written into
/// [`EventEnvelope::topic`] by [`command_event_envelope`].
///
/// Exposing this as a public function lets the CLI capability-offer layer
/// advertise the exact topic without duplicating the sanitization logic.
pub fn command_event_topic(pack_name: &str, event_name: &str) -> String {
    format!(
        "sorla.{}.{}",
        topic_segment(pack_name),
        topic_segment(event_name),
    )
}

/// Parses and validates environment and tenant identifiers into a `TenantCtx`.
fn build_tenant_ctx(environment: &str, tenant_id: &str) -> SorxResult<TenantCtx> {
    let env = environment
        .parse::<EnvId>()
        .map_err(|err| SorxError::new("event_env_invalid", err.to_string()))?;
    let tenant = tenant_id
        .parse::<TenantId>()
        .map_err(|err| SorxError::new("event_tenant_invalid", err.to_string()))?;
    Ok(TenantCtx::new(env, tenant))
}

/// Formats the event source string from a runtime pack descriptor.
fn build_event_source(pack: &RuntimePack) -> String {
    format!("sorx:{}:{}", pack.name, pack.version)
}

/// Builds a canonical envelope for a record-lifecycle event (create, update,
/// delete, and so on).
///
/// The envelope `topic` follows the pattern
/// `sorla.<pack>.<Entity>.<operation>` where each segment is sanitized for
/// NATS routing. The `subject` and payload values are left unsanitized.
///
/// # Parameters
///
/// - `pack` - Runtime descriptor for the Sorla pack emitting the event.
/// - `input` - Structured input containing environment, tenant, entity, operation,
///   record id, optional record snapshot, and optional idempotency key.
///
/// # Errors
///
/// Returns an error when `input.environment` or `input.tenant_id` fail
/// identifier validation.
pub fn entity_event_envelope(
    pack: &RuntimePack,
    input: EntityEventInput<'_>,
) -> SorxResult<EventEnvelope> {
    let mut payload = json!({
        "entity": input.entity,
        "id": input.record_id,
        "operation": input.operation,
    });
    if let (Some(fields), Some(record_value)) = (payload.as_object_mut(), input.record) {
        fields.insert("record".to_string(), record_value);
    }
    let mut metadata = greentic_types::EventMetadata::default();
    if let Some(key) = input.idempotency_key {
        metadata.insert("idempotency_key".to_string(), key.to_owned());
    }
    Ok(EventEnvelope {
        id: next_event_id()?,
        topic: entity_event_topic(&pack.name, input.entity, input.operation),
        r#type: format!("com.greentic.sorla.entity.{}.v1", input.operation),
        source: build_event_source(pack),
        tenant: build_tenant_ctx(input.environment, input.tenant_id)?,
        subject: Some(format!("{}:{}", input.entity, input.record_id)),
        time: Utc::now(),
        correlation_id: input.idempotency_key.map(str::to_owned),
        payload,
        metadata,
    })
}

/// Builds a canonical envelope for a named command or domain event emitted
/// explicitly by a command step (the `emit_event` step kind).
///
/// The envelope `topic` follows the pattern `sorla.<pack>.<EventName>` where
/// each segment is sanitized for NATS routing. The `subject` and payload
/// values are left unsanitized.
///
/// # Parameters
///
/// - `pack` - Runtime descriptor for the Sorla pack emitting the event.
/// - `input` - Structured input containing environment, tenant, event name,
///   subject entity, subject id, payload, and optional idempotency key.
///
/// # Errors
///
/// Returns an error when `input.environment` or `input.tenant_id` fail
/// identifier validation.
pub fn command_event_envelope(
    pack: &RuntimePack,
    input: CommandEventInput<'_>,
) -> SorxResult<EventEnvelope> {
    let mut metadata = greentic_types::EventMetadata::default();
    if let Some(key) = input.idempotency_key {
        metadata.insert("idempotency_key".to_string(), key.to_owned());
    }
    Ok(EventEnvelope {
        id: next_event_id()?,
        topic: command_event_topic(&pack.name, input.event_name),
        r#type: format!("com.greentic.sorla.{}.v1", input.event_name),
        source: build_event_source(pack),
        tenant: build_tenant_ctx(input.environment, input.tenant_id)?,
        subject: Some(format!("{}:{}", input.subject_entity, input.subject_id)),
        time: Utc::now(),
        correlation_id: input.idempotency_key.map(str::to_owned),
        payload: input.payload,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::runtime::runtime_pack;

    fn make_pack(name: &str) -> RuntimePack {
        runtime_pack(name, "1.0.0")
    }

    // -----------------------------------------------------------------------
    // entity_event_envelope
    // -----------------------------------------------------------------------

    #[test]
    fn entity_event_creates_correct_topic_and_subject() {
        let pack = make_pack("landlord");
        let envelope = entity_event_envelope(
            &pack,
            EntityEventInput {
                environment: "local",
                tenant_id: "tenant-1",
                entity: "Tenant",
                operation: "created",
                record_id: "rec-1",
                record: None,
                idempotency_key: None,
            },
        )
        .expect("entity_event_envelope should succeed with valid inputs");

        assert_eq!(envelope.topic, "sorla.landlord.Tenant.created");
        assert_eq!(envelope.subject.as_deref(), Some("Tenant:rec-1"));
        assert_eq!(envelope.r#type, "com.greentic.sorla.entity.created.v1");
    }

    #[test]
    fn entity_event_embeds_record_snapshot_when_provided() {
        let pack = make_pack("landlord");
        let record_value = json!({"name": "Acme"});
        let envelope = entity_event_envelope(
            &pack,
            EntityEventInput {
                environment: "local",
                tenant_id: "tenant-1",
                entity: "Tenant",
                operation: "updated",
                record_id: "rec-2",
                record: Some(record_value.clone()),
                idempotency_key: None,
            },
        )
        .expect("entity_event_envelope should succeed with valid inputs");

        assert_eq!(envelope.payload["record"], record_value);
    }

    #[test]
    fn entity_event_propagates_idempotency_key_to_correlation_and_metadata() {
        let pack = make_pack("landlord");
        let envelope = entity_event_envelope(
            &pack,
            EntityEventInput {
                environment: "local",
                tenant_id: "tenant-1",
                entity: "Tenant",
                operation: "created",
                record_id: "rec-3",
                record: None,
                idempotency_key: Some("idem-1"),
            },
        )
        .expect("entity_event_envelope should succeed with valid inputs");

        assert_eq!(envelope.correlation_id.as_deref(), Some("idem-1"));
        assert_eq!(
            envelope.metadata.get("idempotency_key").map(String::as_str),
            Some("idem-1")
        );
    }

    #[test]
    fn entity_event_sanitizes_topic_segments_but_preserves_subject() {
        let pack = make_pack("landlord");
        let envelope = entity_event_envelope(
            &pack,
            EntityEventInput {
                environment: "local",
                tenant_id: "tenant-1",
                entity: "My Entity.v2",
                operation: "created",
                record_id: "rec-1",
                record: None,
                idempotency_key: None,
            },
        )
        .expect("entity_event_envelope should succeed with valid inputs");

        assert_eq!(envelope.topic, "sorla.landlord.My-Entity-v2.created");
        assert_eq!(envelope.subject.as_deref(), Some("My Entity.v2:rec-1"));
    }

    #[test]
    fn entity_event_returns_error_for_invalid_environment() {
        let pack = make_pack("landlord");
        let result = entity_event_envelope(
            &pack,
            EntityEventInput {
                environment: "invalid env!",
                tenant_id: "tenant-1",
                entity: "Tenant",
                operation: "created",
                record_id: "rec-1",
                record: None,
                idempotency_key: None,
            },
        );

        assert!(
            result.is_err(),
            "entity_event_envelope should fail for an invalid environment identifier"
        );
    }

    #[test]
    fn entity_event_with_empty_record_id_documents_current_subject_format() {
        let pack = make_pack("landlord");
        let envelope = entity_event_envelope(
            &pack,
            EntityEventInput {
                environment: "local",
                tenant_id: "tenant-1",
                entity: "E",
                operation: "created",
                record_id: "",
                record: None,
                idempotency_key: None,
            },
        )
        .expect("entity_event_envelope should succeed even with an empty record_id");

        // Current behaviour: subject is "<entity>:" when record_id is empty.
        assert_eq!(envelope.subject.as_deref(), Some("E:"));
    }

    // -----------------------------------------------------------------------
    // command_event_envelope
    // -----------------------------------------------------------------------

    #[test]
    fn command_event_creates_correct_topic_and_subject() {
        let pack = make_pack("landlord");
        let envelope = command_event_envelope(
            &pack,
            CommandEventInput {
                environment: "local",
                tenant_id: "tenant-1",
                event_name: "RecordRemoved",
                subject_entity: "Tenant",
                subject_id: "rec-4",
                payload: json!({"reason": "deactivated"}),
                idempotency_key: None,
            },
        )
        .expect("command_event_envelope should succeed with valid inputs");

        assert_eq!(envelope.topic, "sorla.landlord.RecordRemoved");
        assert_eq!(envelope.subject.as_deref(), Some("Tenant:rec-4"));
        assert_eq!(envelope.r#type, "com.greentic.sorla.RecordRemoved.v1");
    }

    #[test]
    fn command_event_propagates_idempotency_key_to_correlation_and_metadata() {
        let pack = make_pack("landlord");
        let envelope = command_event_envelope(
            &pack,
            CommandEventInput {
                environment: "local",
                tenant_id: "tenant-1",
                event_name: "RecordRemoved",
                subject_entity: "Tenant",
                subject_id: "rec-5",
                payload: json!({}),
                idempotency_key: Some("idem-2"),
            },
        )
        .expect("command_event_envelope should succeed with valid inputs");

        assert_eq!(envelope.correlation_id.as_deref(), Some("idem-2"));
        assert_eq!(
            envelope.metadata.get("idempotency_key").map(String::as_str),
            Some("idem-2")
        );
    }
}
