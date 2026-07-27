//! Best-effort boot-time `SorxPresence` NATS publish hook.
//!
//! Producer half of epic #3 NATS push-discovery: on `start`, once the HTTP
//! runtime is bound and its `base_url` is known, the runtime announces
//! itself once on `greentic.presence.<tenant>.sorx.<sor>` so consumers can
//! discover it without a directory lookup. This is a boot-time announcement
//! only; periodic re-publish (heartbeat) is out of scope for this slice.
//!
//! Mirrors [`crate::nats_events::NatsEventSink::connect`]'s connection
//! pattern: a dedicated OS thread runs a current-thread tokio runtime so the
//! synchronous HTTP serve loop is never blocked or affected. Every failure
//! (spawn, connect, encode, publish, flush) logs a warning and returns —
//! this hook must never panic or fail boot.
//!
//! The actual publish is compiled only under the `presence-nats` feature;
//! with the feature off, [`publish_presence_on_boot`] is a no-op stub with
//! the same signature so callers do not need to `cfg`-gate the call site.
//! Even with the feature on, the hook is inert unless `SORX_PRESENCE_NATS_URL`
//! is set in the environment.

use greentic_sorx_core::RuntimeCapabilities;
use greentic_sorx_core::presence::{SORX_PRESENCE_SCHEMA, SorxPresence};

/// Environment variable that enables the boot-publish hook at runtime.
///
/// Mirrors the `GREENTIC_EVENTS_NATS_URL` / event-bridge pattern: the
/// dependency is compiled in via the `presence-nats` feature, but the hook
/// stays inert until this variable names a NATS server URL.
#[cfg(feature = "presence-nats")]
const SORX_PRESENCE_NATS_URL_ENV: &str = "SORX_PRESENCE_NATS_URL";

/// Sanitizes a value for use as a single NATS subject segment.
///
/// Only ASCII alphanumeric characters, hyphens, and underscores are kept.
/// Any other character (including spaces and dots) is replaced with a
/// hyphen. An empty result falls back to `"unknown"`. Mirrors
/// `greentic_sorx_core::business_events::topic_segment`, vendored locally
/// because that function is private to its module.
fn sanitize_segment(value: &str) -> String {
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

/// Returns the NATS subject a boot-time `SorxPresence` is published on:
/// `greentic.presence.<tenant>.sorx.<sor>`, with each segment sanitized by
/// [`sanitize_segment`].
pub(crate) fn presence_subject(tenant: &str, sor: &str) -> String {
    format!(
        "greentic.presence.{}.sorx.{}",
        sanitize_segment(tenant),
        sanitize_segment(sor),
    )
}

/// Builds the `SorxPresence` payload to announce at boot.
///
/// Pure and side-effect free so it can be unit tested without a NATS
/// connection. `reachable` mirrors whether the operator configured a
/// `server.public_base_url` (`public_base_url_set`): without one, `base_url`
/// is a loopback/bind address that only the local host can reach.
///
/// `instance_id` combines the OS process id with the caller-supplied
/// timestamp so repeated restarts of the same pack/tenant/sor in the same
/// process lifetime still yield a stable, collision-resistant identifier —
/// the same pattern used for event ids in
/// `greentic_sorx_core::business_events::process_tag`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_presence(
    tenant: &str,
    environment: &str,
    sor: &str,
    pack_version: &str,
    base_url: &str,
    public_base_url_set: bool,
    offers: RuntimeCapabilities,
    ts: String,
) -> SorxPresence {
    let instance_id = format!("sorx-{:x}-{ts}", std::process::id());
    SorxPresence {
        schema: SORX_PRESENCE_SCHEMA.to_string(),
        instance_id,
        tenant: tenant.to_string(),
        environment: environment.to_string(),
        sor: sor.to_string(),
        pack_version: pack_version.to_string(),
        base_url: base_url.to_string(),
        reachable: public_base_url_set,
        offers,
        ts,
    }
}

/// Publishes `presence` on `subject` over NATS, best-effort, on a dedicated
/// background thread.
///
/// Returns immediately after (attempting to) spawn the background thread;
/// never blocks the caller. Reads `SORX_PRESENCE_NATS_URL` and returns
/// immediately without spawning anything when it is unset. Every failure —
/// thread spawn, connect, encode, publish, or flush — logs a warning and
/// returns; this function never panics and never propagates an error, so a
/// broken or absent NATS server can never fail boot.
#[cfg(feature = "presence-nats")]
pub(crate) fn publish_presence_on_boot(presence: SorxPresence, subject: String) {
    let Ok(url) = std::env::var(SORX_PRESENCE_NATS_URL_ENV) else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("sorx-presence-publish".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    eprintln!("sorx presence: failed to start publish runtime: {err}");
                    return;
                }
            };
            runtime.block_on(async move {
                let client = match async_nats::connect(&url).await {
                    Ok(client) => client,
                    Err(err) => {
                        eprintln!("sorx presence: nats connect failed ({url}): {err}");
                        return;
                    }
                };
                let bytes = match serde_json::to_vec(&presence) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        eprintln!("sorx presence: envelope encode failed: {err}");
                        return;
                    }
                };
                if let Err(err) = client.publish(subject, bytes.into()).await {
                    eprintln!("sorx presence: nats publish failed: {err}");
                    return;
                }
                if let Err(err) = client.flush().await {
                    eprintln!("sorx presence: nats flush failed: {err}");
                }
            });
        });
    if let Err(err) = spawned {
        eprintln!("sorx presence: failed to spawn publish thread: {err}");
    }
}

/// No-op stub used when the `presence-nats` feature is disabled: drops the
/// presence and subject and does nothing. Keeps the call site in `run_start`
/// feature-agnostic.
#[cfg(not(feature = "presence-nats"))]
pub(crate) fn publish_presence_on_boot(_presence: SorxPresence, _subject: String) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_offers() -> RuntimeCapabilities {
        RuntimeCapabilities::sorx_runtime_host()
    }

    #[test]
    fn presence_subject_sanitizes_segments() {
        assert_eq!(
            presence_subject("acme co.", "landlord tenant"),
            "greentic.presence.acme-co-.sorx.landlord-tenant"
        );
    }

    #[test]
    fn presence_subject_builds_plain_segments_unchanged() {
        assert_eq!(
            presence_subject("acme", "landlord-tenant-sor"),
            "greentic.presence.acme.sorx.landlord-tenant-sor"
        );
    }

    #[test]
    fn build_presence_sets_reachable_from_public_base_url() {
        let reachable = build_presence(
            "acme",
            "local",
            "landlord-tenant-sor",
            "0.1.0",
            "https://sor.acme.example",
            true,
            make_offers(),
            "2026-07-27T00:00:00Z".to_string(),
        );
        assert!(reachable.reachable);

        let not_reachable = build_presence(
            "acme",
            "local",
            "landlord-tenant-sor",
            "0.1.0",
            "http://127.0.0.1:8787",
            false,
            make_offers(),
            "2026-07-27T00:00:00Z".to_string(),
        );
        assert!(!not_reachable.reachable);
    }

    #[test]
    fn build_presence_sets_schema_and_stable_instance_id() {
        let presence = build_presence(
            "acme",
            "local",
            "landlord-tenant-sor",
            "0.1.0",
            "http://127.0.0.1:8787",
            false,
            make_offers(),
            "2026-07-27T00:00:00Z".to_string(),
        );
        assert_eq!(presence.schema, SORX_PRESENCE_SCHEMA);
        assert!(!presence.instance_id.is_empty());
        assert!(presence.instance_id.starts_with("sorx-"));
        assert!(presence.instance_id.ends_with("2026-07-27T00:00:00Z"));
        // Same pid + timestamp must always yield the same instance_id.
        let again = build_presence(
            "acme",
            "local",
            "landlord-tenant-sor",
            "0.1.0",
            "http://127.0.0.1:8787",
            false,
            make_offers(),
            "2026-07-27T00:00:00Z".to_string(),
        );
        assert_eq!(presence.instance_id, again.instance_id);
    }

    #[test]
    fn build_presence_copies_scalar_fields() {
        let presence = build_presence(
            "acme",
            "local",
            "landlord-tenant-sor",
            "0.1.0",
            "https://sor.acme.example",
            true,
            make_offers(),
            "2026-07-27T00:00:00Z".to_string(),
        );
        assert_eq!(presence.tenant, "acme");
        assert_eq!(presence.environment, "local");
        assert_eq!(presence.sor, "landlord-tenant-sor");
        assert_eq!(presence.pack_version, "0.1.0");
        assert_eq!(presence.base_url, "https://sor.acme.example");
        assert_eq!(presence.ts, "2026-07-27T00:00:00Z");
    }

    /// Live-NATS round trip: connects a subscriber, publishes via the boot
    /// hook, and asserts it receives + decodes the `SorxPresence`. Requires a
    /// real `nats-server` reachable at `SORX_PRESENCE_NATS_URL` (defaults to
    /// `nats://127.0.0.1:4222`), so it is `#[ignore]`d — `--all-features`
    /// local_check runs skip it without a broker.
    ///
    /// Run with a live server:
    ///
    /// ```text
    /// SORX_PRESENCE_NATS_URL=nats://127.0.0.1:4222 \
    ///   cargo test -p greentic-sorx --features presence-nats \
    ///   presence_publish::tests::publish_presence_on_boot_round_trips_over_nats \
    ///   -- --ignored --nocapture
    /// ```
    #[cfg(feature = "presence-nats")]
    #[tokio::test]
    #[ignore]
    async fn publish_presence_on_boot_round_trips_over_nats() {
        let nats_url = std::env::var(SORX_PRESENCE_NATS_URL_ENV)
            .unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
        // SAFETY-equivalent: single-threaded async test, set before any other
        // code reads this variable.
        // The hook itself re-reads this exact variable at publish time.
        // (Setting it here keeps the test self-contained when the caller
        // did not export SORX_PRESENCE_NATS_URL explicitly.)
        unsafe {
            std::env::set_var(SORX_PRESENCE_NATS_URL_ENV, &nats_url);
        }

        let client = async_nats::connect(&nats_url)
            .await
            .expect("connect to nats-server for the live round-trip test");
        let subject = presence_subject("acme", "landlord-tenant-sor");
        let mut subscription = client
            .subscribe(subject.clone())
            .await
            .expect("subscribe to presence subject");

        let presence = build_presence(
            "acme",
            "local",
            "landlord-tenant-sor",
            "0.1.0",
            "https://sor.acme.example",
            true,
            RuntimeCapabilities::sorx_runtime_host(),
            "2026-07-27T00:00:00Z".to_string(),
        );
        publish_presence_on_boot(presence.clone(), subject.clone());

        use futures::StreamExt;
        let message = tokio::time::timeout(std::time::Duration::from_secs(5), subscription.next())
            .await
            .expect("timed out waiting for presence — nats-server may be unreachable")
            .expect("subscription closed before receiving the presence message");

        assert_eq!(message.subject.as_str(), subject);
        let decoded: SorxPresence =
            serde_json::from_slice(&message.payload).expect("decode SorxPresence payload");
        assert_eq!(decoded, presence);
    }
}
