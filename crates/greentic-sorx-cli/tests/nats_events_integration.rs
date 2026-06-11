//! Gated NATS integration test: round-trips one envelope through a real NATS
//! server.  The test skips (passes) when `NATS_URL` is not set in the
//! environment, mirroring the greentic-integration pattern.
//!
//! Run with a live server:
//!
//! ```text
//! NATS_URL=nats://127.0.0.1:4222 \
//!   cargo test -p greentic-sorx --features events-nats --test nats_events_integration -- --nocapture
//! ```

#![cfg(feature = "events-nats")]

use futures::StreamExt;
use greentic_sorx_cli::nats_events::NatsEventSink;
use greentic_sorx_core::{
    BusinessEventSink, EntityEventInput, entity_event_envelope, runtime_pack,
};

#[test]
fn published_envelope_arrives_on_nats_subject() {
    let Ok(nats_url) = std::env::var("NATS_URL") else {
        eprintln!("skipping: NATS_URL not set");
        return;
    };

    let pack = runtime_pack("landlord", "0.1.0");
    let envelope = entity_event_envelope(
        &pack,
        EntityEventInput {
            environment: "local",
            tenant_id: "tenant-it",
            entity: "Tenant",
            operation: "created",
            record_id: "rec-it-1",
            record: Some(serde_json::json!({"id": "rec-it-1"})),
            idempotency_key: None,
        },
    )
    .unwrap();

    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let received_message = tokio_runtime.block_on(async {
        let client = async_nats::connect(&nats_url).await.unwrap();
        let mut subscription = client
            .subscribe("greentic.events.tenant-it.>")
            .await
            .unwrap();

        let sink = NatsEventSink::connect(&nats_url, "greentic.events").unwrap();
        sink.publish(envelope.clone()).unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            subscription.next().await
        })
        .await
        .expect("timed out waiting for event — NATS may be unreachable or the sink stalled")
        .expect("subscription closed before receiving any message")
    });

    let parsed: greentic_types::EventEnvelope =
        serde_json::from_slice(&received_message.payload).unwrap();

    assert_eq!(parsed.topic, "sorla.landlord.Tenant.created");
    assert_eq!(parsed.tenant.tenant.as_str(), "tenant-it");
    assert_eq!(
        received_message.subject.as_str(),
        "greentic.events.tenant-it.sorla.landlord.Tenant.created"
    );
}
