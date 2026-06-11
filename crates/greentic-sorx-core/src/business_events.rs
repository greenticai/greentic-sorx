//! Business event publication: maps Sorx record/command activity onto the
//! canonical Greentic event envelope and hands it to a pluggable sink.
//!
//! Mirrors the `audit` module pattern. Publication is best-effort: callers
//! must never fail a business operation because a sink failed.

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

/// Monotonically increasing nonce combined with a millisecond timestamp to
/// produce unique, lexicographically ordered event identifiers.
static EVENT_NONCE: AtomicU64 = AtomicU64::new(1);

/// Generates the next unique event identifier.
fn next_event_id() -> SorxResult<EventId> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let nonce = EVENT_NONCE.fetch_add(1, Ordering::Relaxed);
    EventId::new(format!("sorla-{millis}-{nonce}"))
        .map_err(|err| SorxError::new("event_id_invalid", err.to_string()))
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
/// The envelope `topic` follows the pattern `sorla.<pack>.<Entity>.<operation>`,
/// and the `payload` contains `entity`, `id`, `operation`, and an optional
/// `record` field when the caller provides a JSON record snapshot.
///
/// # Parameters
///
/// - `pack` - Runtime descriptor for the Sorla pack emitting the event.
/// - `environment` - Environment identifier (for example `local`, `prod`).
/// - `tenant_id` - Tenant identifier string; must be a valid Greentic identifier.
/// - `entity` - Entity type name (for example `Tenant`).
/// - `operation` - Operation label (for example `created`, `updated`, `deleted`).
/// - `record_id` - Stable identifier of the affected record.
/// - `record` - Optional full record snapshot to embed in the payload.
/// - `idempotency_key` - Optional key forwarded as `correlation_id`.
///
/// # Errors
///
/// Returns an error when `environment` or `tenant_id` fail identifier
/// validation.
#[allow(clippy::too_many_arguments)]
pub fn entity_event_envelope(
    pack: &RuntimePack,
    environment: &str,
    tenant_id: &str,
    entity: &str,
    operation: &str,
    record_id: &str,
    record: Option<Value>,
    idempotency_key: Option<&str>,
) -> SorxResult<EventEnvelope> {
    let mut payload = json!({
        "entity": entity,
        "id": record_id,
        "operation": operation,
    });
    if let (Some(fields), Some(record_value)) = (payload.as_object_mut(), record) {
        fields.insert("record".to_string(), record_value);
    }
    Ok(EventEnvelope {
        id: next_event_id()?,
        topic: format!("sorla.{}.{entity}.{operation}", pack.name),
        r#type: format!("com.greentic.sorla.entity.{operation}.v1"),
        source: build_event_source(pack),
        tenant: build_tenant_ctx(environment, tenant_id)?,
        subject: Some(format!("{entity}:{record_id}")),
        time: Utc::now(),
        correlation_id: idempotency_key.map(str::to_owned),
        payload,
        metadata: Default::default(),
    })
}

/// Builds a canonical envelope for a named command or domain event emitted
/// explicitly by a command step (the `emit_event` step kind).
///
/// The envelope `topic` follows the pattern `sorla.<pack>.<EventName>`, and
/// the caller provides the full `payload` directly.
///
/// # Parameters
///
/// - `pack` - Runtime descriptor for the Sorla pack emitting the event.
/// - `environment` - Environment identifier string.
/// - `tenant_id` - Tenant identifier string; must be a valid Greentic identifier.
/// - `event_name` - Stable event name (for example `RecordRemoved`).
/// - `subject_entity` - Entity type that the event concerns.
/// - `subject_id` - Record identifier that forms the `subject` field.
/// - `payload` - Full event payload as a JSON value.
/// - `idempotency_key` - Optional key forwarded as `correlation_id`.
///
/// # Errors
///
/// Returns an error when `environment` or `tenant_id` fail identifier
/// validation.
#[allow(clippy::too_many_arguments)]
pub fn command_event_envelope(
    pack: &RuntimePack,
    environment: &str,
    tenant_id: &str,
    event_name: &str,
    subject_entity: &str,
    subject_id: &str,
    payload: Value,
    idempotency_key: Option<&str>,
) -> SorxResult<EventEnvelope> {
    Ok(EventEnvelope {
        id: next_event_id()?,
        topic: format!("sorla.{}.{event_name}", pack.name),
        r#type: format!("com.greentic.sorla.{event_name}.v1"),
        source: build_event_source(pack),
        tenant: build_tenant_ctx(environment, tenant_id)?,
        subject: Some(format!("{subject_entity}:{subject_id}")),
        time: Utc::now(),
        correlation_id: idempotency_key.map(str::to_owned),
        payload,
        metadata: Default::default(),
    })
}
