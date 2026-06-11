//! NATS-backed business event sink.
//!
//! Bridges the synchronous Sorx runtime to async-nats: `publish` enqueues into
//! a bounded channel and returns immediately; a dedicated thread running a
//! current-thread tokio runtime drains the queue and publishes JSON envelopes.
//!
//! # Lazy construction
//!
//! Construction is lazy with respect to NATS availability: `connect` only
//! fails if the publisher thread cannot spawn. If NATS is unreachable, events
//! are dropped with a log line and the runtime keeps serving. Delivery is
//! at-most-once: a full queue or NATS outage drops events (records remain the
//! source of truth in the canonical store).

use std::thread;

use greentic_sorx_core::{BusinessEventSink, SorxError, SorxResult};
use greentic_types::EventEnvelope;
use tokio::sync::mpsc::{Sender, error::TrySendError};

/// Maximum number of envelopes that can be queued before drops begin.
const QUEUE_CAPACITY: usize = 1024;

/// NATS-backed business event sink.
///
/// Holds a channel sender that routes envelopes to a background thread which
/// runs a single-threaded tokio runtime and drains the queue asynchronously.
/// The struct itself is sync and cheap to clone or share behind an `Arc`.
pub struct NatsEventSink {
    sender: Sender<EventEnvelope>,
    subject_prefix: String,
}

/// Builds the NATS subject string from prefix, tenant, and topic segments.
///
/// Subject follows the pattern `<prefix>.<tenant>.<topic>`, which allows NATS
/// subject-based routing and fan-out via wildcards.
fn nats_subject(prefix: &str, tenant: &str, topic: &str) -> String {
    format!("{prefix}.{tenant}.{topic}")
}

impl NatsEventSink {
    /// Constructs a new `NatsEventSink` and spawns the background publisher thread.
    ///
    /// The background thread connects to NATS at `url` asynchronously. If the
    /// connection fails, events are silently dropped with a log line and the
    /// runtime continues serving.  The thread exits gracefully when all senders
    /// are dropped (i.e. when the runtime shuts down).
    ///
    /// # Parameters
    ///
    /// - `url` - NATS server URL, for example `nats://localhost:4222`.
    /// - `subject_prefix` - Prefix prepended to all published subjects.
    ///
    /// # Errors
    ///
    /// Returns `SorxError` only when the OS refuses to spawn the thread.
    pub fn connect(url: &str, subject_prefix: &str) -> SorxResult<Self> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<EventEnvelope>(QUEUE_CAPACITY);
        let url = url.to_string();
        let prefix = subject_prefix.to_string();
        thread::Builder::new()
            .name("sorx-nats-events".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        eprintln!("sorx events: failed to start nats publisher runtime: {err}");
                        return;
                    }
                };
                runtime.block_on(async move {
                    let client = match async_nats::connect(&url).await {
                        Ok(client) => client,
                        Err(err) => {
                            eprintln!("sorx events: nats connect failed ({url}): {err}");
                            return;
                        }
                    };
                    while let Some(envelope) = receiver.recv().await {
                        let subject =
                            nats_subject(&prefix, envelope.tenant.tenant.as_str(), &envelope.topic);
                        match serde_json::to_vec(&envelope) {
                            Ok(bytes) => {
                                if let Err(err) = client.publish(subject, bytes.into()).await {
                                    eprintln!("sorx events: nats publish failed: {err}");
                                }
                            }
                            Err(err) => {
                                eprintln!("sorx events: envelope encode failed: {err}");
                            }
                        }
                    }
                });
            })
            .map_err(|err| SorxError::new("events_nats_thread_failed", err.to_string()))?;
        Ok(Self {
            sender,
            subject_prefix: subject_prefix.to_string(),
        })
    }

    /// Constructs a `NatsEventSink` from an existing channel sender.
    ///
    /// Intended for unit tests that want to inspect enqueued envelopes without
    /// spinning up a real NATS connection.
    #[cfg(test)]
    fn with_sender(sender: Sender<EventEnvelope>, subject_prefix: String) -> Self {
        Self {
            sender,
            subject_prefix,
        }
    }
}

impl BusinessEventSink for NatsEventSink {
    /// Enqueues the envelope for async delivery and returns immediately.
    ///
    /// If the queue is full or the publisher thread has stopped, the envelope
    /// is dropped with a log line and `Ok(())` is returned.  Callers must
    /// never fail a business operation because a sink could not deliver.
    fn publish(&self, envelope: EventEnvelope) -> SorxResult<()> {
        match self.sender.try_send(envelope) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(dropped)) => {
                eprintln!(
                    "sorx events: queue full, dropping event {} (prefix {})",
                    dropped.topic, self.subject_prefix
                );
                Ok(())
            }
            Err(TrySendError::Closed(dropped)) => {
                eprintln!(
                    "sorx events: publisher stopped, dropping event {}",
                    dropped.topic
                );
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_sorx_core::BusinessEventSink;
    use greentic_types::{EnvId, EventEnvelope, EventId, TenantCtx, TenantId};
    use serde_json::json;

    fn make_envelope(topic: &str) -> EventEnvelope {
        EventEnvelope {
            id: EventId::new("evt-1").unwrap(),
            topic: topic.to_string(),
            r#type: "com.greentic.sorla.test.v1".to_string(),
            source: "sorx:test:0".to_string(),
            tenant: TenantCtx::new(
                "local".parse::<EnvId>().unwrap(),
                "tenant-a".parse::<TenantId>().unwrap(),
            ),
            subject: None,
            time: chrono::Utc::now(),
            correlation_id: None,
            payload: json!({}),
            metadata: Default::default(),
        }
    }

    #[test]
    fn publish_enqueues_and_never_blocks() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
        let sink = NatsEventSink::with_sender(sender, "greentic.events".to_string());
        sink.publish(make_envelope("sorla.p.E.created")).unwrap();
        let queued = receiver.try_recv().unwrap();
        assert_eq!(queued.topic, "sorla.p.E.created");
    }

    #[test]
    fn publish_drops_when_queue_full_without_error() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        let sink = NatsEventSink::with_sender(sender, "greentic.events".to_string());
        sink.publish(make_envelope("a")).unwrap();
        sink.publish(make_envelope("b")).unwrap(); // full queue must drop silently
    }

    #[test]
    fn publish_after_worker_stop_is_ok() {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(receiver);
        let sink = NatsEventSink::with_sender(sender, "greentic.events".to_string());
        sink.publish(make_envelope("a")).unwrap();
    }

    #[test]
    fn subject_is_prefix_tenant_topic() {
        assert_eq!(
            nats_subject("greentic.events", "tenant-a", "sorla.p.E.created"),
            "greentic.events.tenant-a.sorla.p.E.created"
        );
    }
}
