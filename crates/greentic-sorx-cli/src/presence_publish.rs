//! Best-effort boot-time `SorxPresence` NATS publish hook, plus an optional
//! periodic heartbeat re-publish.
//!
//! Producer half of epic #3 NATS push-discovery: on `start`, once the HTTP
//! runtime is bound and its `base_url` is known, the runtime announces
//! itself once on `greentic.presence.<tenant>.sorx.<sor>` so consumers can
//! discover it without a directory lookup. A periodic heartbeat
//! (`spawn_presence_heartbeat`) then re-publishes the same presence (with a
//! fresh `ts`) on an interval so consumers' soft-state directories don't
//! evict a still-live instance.
//!
//! Mirrors [`crate::nats_events::NatsEventSink::connect`]'s connection
//! pattern: a dedicated OS thread runs a current-thread tokio runtime so the
//! synchronous HTTP serve loop is never blocked or affected. Every failure
//! (spawn, connect, encode, publish, flush) logs a warning and continues —
//! neither hook must ever panic or fail boot.
//!
//! The actual publish/heartbeat is compiled only under the `presence-nats`
//! feature; with the feature off, [`publish_presence_on_boot`] and
//! [`spawn_presence_heartbeat`] are no-op stubs with the same signatures so
//! callers do not need to `cfg`-gate the call sites. Even with the feature
//! on, both hooks are inert unless `SORX_PRESENCE_NATS_URL` is set in the
//! environment.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use greentic_sorx_core::RuntimeCapabilities;
use greentic_sorx_core::presence::{SORX_PRESENCE_SCHEMA, SorxPresence};

/// Environment variable that enables the boot-publish and heartbeat hooks at
/// runtime.
///
/// Mirrors the `GREENTIC_EVENTS_NATS_URL` / event-bridge pattern: the
/// dependency is compiled in via the `presence-nats` feature, but the hooks
/// stay inert until this variable names a NATS server URL.
///
/// Not feature-gated: [`spawn_presence_heartbeat_if_enabled`] reads it
/// unconditionally (and is itself the always-compiled entry point `run_start`
/// calls) so the "enabled" check runs the same way whether or not
/// `presence-nats` was compiled in.
pub(crate) const SORX_PRESENCE_NATS_URL_ENV: &str = "SORX_PRESENCE_NATS_URL";

/// Environment variable controlling the heartbeat re-publish interval, in
/// seconds. See [`heartbeat_interval_from_env`] for parsing rules.
pub(crate) const SORX_PRESENCE_HEARTBEAT_SECS_ENV: &str = "SORX_PRESENCE_HEARTBEAT_SECS";

/// Default heartbeat interval, in seconds, used when the env var is unset or
/// fails to parse.
const DEFAULT_HEARTBEAT_SECS: u64 = 30;

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

/// Process-stable presence `instance_id`, computed once per process and
/// cached.
///
/// Mirrors `greentic_sorx_core::business_events::process_tag`: mixes the OS
/// process id with the subsecond nanoseconds of first use, so two replicas
/// (or restarts) that land in the same millisecond still get distinct ids.
/// Unlike a `ts`-derived id, this value is computed exactly once per process
/// and never changes afterwards — required so a heartbeat re-publish keeps
/// identifying the same instance while only its `ts` field moves forward.
/// Not `presence-nats`-gated so `build_presence` behaves identically with
/// the feature on or off.
static PROCESS_INSTANCE_ID: OnceLock<String> = OnceLock::new();

/// Returns the process-stable presence `instance_id`, computing it once on
/// first call and returning the cached value thereafter.
///
/// Pure with respect to arguments (there are none) but memoized as process
/// state, exactly like `business_events::process_tag`. Always available
/// (not `presence-nats`-gated) so `build_presence` and its unit tests behave
/// identically with the feature on or off.
pub(crate) fn process_instance_id() -> String {
    PROCESS_INSTANCE_ID
        .get_or_init(|| {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.subsec_nanos())
                .unwrap_or_default();
            format!("sorx-{:x}-{:x}", std::process::id(), nanos)
        })
        .clone()
}

/// Builds the `SorxPresence` payload to announce at boot (and, unchanged
/// apart from `ts`, on every heartbeat re-publish).
///
/// Pure and side-effect free so it can be unit tested without a NATS
/// connection. `reachable` mirrors whether the operator configured a
/// `server.public_base_url` (`public_base_url_set`): without one, `base_url`
/// is a loopback/bind address that only the local host can reach.
///
/// `instance_id` is [`process_instance_id`]: stable for the lifetime of the
/// process, so repeated calls (boot announcement, then each heartbeat tick)
/// with a different `ts` still identify the same instance. `ts` is the only
/// field that is expected to differ between calls — it is the publish
/// timestamp, refreshed by the caller (or by
/// [`spawn_presence_heartbeat`]) on every republish.
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
    SorxPresence {
        schema: SORX_PRESENCE_SCHEMA.to_string(),
        instance_id: process_instance_id(),
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

/// Parses `SORX_PRESENCE_HEARTBEAT_SECS` into a heartbeat interval.
///
/// Pure — takes the already-read env var value (or `None` when unset) rather
/// than reading the environment itself, so it is unit-testable without
/// `std::env` mutation. Parsing is defensive: an absent or unparseable value
/// falls back to [`DEFAULT_HEARTBEAT_SECS`] (30s) rather than erroring or
/// disabling the heartbeat. An explicit `"0"` (or anything that parses to
/// `0`) disables the heartbeat entirely, returning `None`.
pub(crate) fn heartbeat_interval_from_env(var: Option<String>) -> Option<Duration> {
    let secs = match var {
        Some(raw) => raw.trim().parse::<u64>().unwrap_or(DEFAULT_HEARTBEAT_SECS),
        None => DEFAULT_HEARTBEAT_SECS,
    };
    if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    }
}

/// Spawns a best-effort background heartbeat that re-publishes `presence` on
/// `subject` every `interval`, refreshing only its `ts` field on each tick.
///
/// Returns immediately after (attempting to) spawn the background thread;
/// never blocks the caller. Reads `SORX_PRESENCE_NATS_URL` and returns
/// immediately without spawning anything when it is unset — callers should
/// still guard the call site with the same check (see `run_start`) to avoid
/// spawning an OS thread that would immediately no-op.
///
/// Connects to NATS once, then loops `sleep(interval)` -> refresh `ts` ->
/// publish -> flush for the lifetime of the process. A publish or flush
/// failure logs a warning and the loop continues on the next tick, reusing
/// the same client — `async-nats` reconnects its underlying connection
/// automatically, so a transient broker outage self-heals without this hook
/// tearing down and reconnecting explicitly. This function never panics and
/// never returns an error; a broken or absent NATS server can never fail
/// boot or crash the heartbeat thread.
#[cfg(feature = "presence-nats")]
pub(crate) fn spawn_presence_heartbeat(
    mut presence: SorxPresence,
    subject: String,
    interval: Duration,
) {
    let Ok(url) = std::env::var(SORX_PRESENCE_NATS_URL_ENV) else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("sorx-presence-heartbeat".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    eprintln!("sorx presence heartbeat: failed to start runtime: {err}");
                    return;
                }
            };
            runtime.block_on(async move {
                let client = match async_nats::connect(&url).await {
                    Ok(client) => client,
                    Err(err) => {
                        eprintln!(
                            "sorx presence heartbeat: nats connect failed ({url}), heartbeat disabled: {err}"
                        );
                        return;
                    }
                };
                loop {
                    tokio::time::sleep(interval).await;
                    presence.ts = chrono::Utc::now().to_rfc3339();
                    let bytes = match serde_json::to_vec(&presence) {
                        Ok(bytes) => bytes,
                        Err(err) => {
                            eprintln!("sorx presence heartbeat: envelope encode failed: {err}");
                            continue;
                        }
                    };
                    if let Err(err) = client.publish(subject.clone(), bytes.into()).await {
                        eprintln!("sorx presence heartbeat: nats publish failed: {err}");
                        continue;
                    }
                    if let Err(err) = client.flush().await {
                        eprintln!("sorx presence heartbeat: nats flush failed: {err}");
                    }
                }
            });
        });
    if let Err(err) = spawned {
        eprintln!("sorx presence heartbeat: failed to spawn heartbeat thread: {err}");
    }
}

/// No-op stub used when the `presence-nats` feature is disabled: drops the
/// presence, subject, and interval and does nothing. Keeps the call site in
/// `run_start` feature-agnostic.
#[cfg(not(feature = "presence-nats"))]
pub(crate) fn spawn_presence_heartbeat(
    _presence: SorxPresence,
    _subject: String,
    _interval: Duration,
) {
}

/// Spawns the presence heartbeat if it is enabled, or does nothing.
///
/// Always compiled (unlike [`spawn_presence_heartbeat`], which is
/// `presence-nats`-gated with a no-op stub) so the call site in `run_start`
/// never needs its own `#[cfg]` — the same pattern [`publish_presence_on_boot`]
/// already uses. "Enabled" means both: `SORX_PRESENCE_NATS_URL` is set (the
/// same gate the boot announcement uses), and
/// [`heartbeat_interval_from_env`] of `SORX_PRESENCE_HEARTBEAT_SECS` did not
/// resolve to "disabled" (an explicit `0`). When disabled, `presence` and
/// `subject` are dropped without spawning anything.
pub(crate) fn spawn_presence_heartbeat_if_enabled(presence: SorxPresence, subject: String) {
    if std::env::var(SORX_PRESENCE_NATS_URL_ENV).is_err() {
        return;
    }
    let Some(interval) =
        heartbeat_interval_from_env(std::env::var(SORX_PRESENCE_HEARTBEAT_SECS_ENV).ok())
    else {
        return;
    };
    spawn_presence_heartbeat(presence, subject, interval);
}

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
        // Same pid must always yield the same instance_id, and it must not
        // be derived from `ts` at all.
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
    fn process_instance_id_is_stable_across_repeated_calls() {
        let first = process_instance_id();
        let second = process_instance_id();
        assert_eq!(first, second);
        assert!(first.starts_with("sorx-"));
    }

    #[test]
    fn build_presence_keeps_instance_id_stable_across_different_ts() {
        let earlier = build_presence(
            "acme",
            "local",
            "landlord-tenant-sor",
            "0.1.0",
            "http://127.0.0.1:8787",
            false,
            make_offers(),
            "2026-07-27T00:00:00Z".to_string(),
        );
        let later = build_presence(
            "acme",
            "local",
            "landlord-tenant-sor",
            "0.1.0",
            "http://127.0.0.1:8787",
            false,
            make_offers(),
            "2026-07-27T00:05:00Z".to_string(),
        );
        // instance_id is process-stable regardless of ts...
        assert_eq!(earlier.instance_id, later.instance_id);
        // ...but ts itself must still change per call: it is the
        // publish-timestamp field a heartbeat refreshes on every tick.
        assert_ne!(earlier.ts, later.ts);
        assert_eq!(earlier.ts, "2026-07-27T00:00:00Z");
        assert_eq!(later.ts, "2026-07-27T00:05:00Z");
    }

    #[test]
    fn heartbeat_interval_from_env_defaults_to_thirty_seconds() {
        assert_eq!(
            heartbeat_interval_from_env(None),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn heartbeat_interval_from_env_zero_disables_heartbeat() {
        assert_eq!(heartbeat_interval_from_env(Some("0".to_string())), None);
    }

    #[test]
    fn heartbeat_interval_from_env_unparseable_falls_back_to_default() {
        assert_eq!(
            heartbeat_interval_from_env(Some("not-a-number".to_string())),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            heartbeat_interval_from_env(Some("".to_string())),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            heartbeat_interval_from_env(Some("-5".to_string())),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn heartbeat_interval_from_env_honors_explicit_value() {
        assert_eq!(
            heartbeat_interval_from_env(Some("45".to_string())),
            Some(Duration::from_secs(45))
        );
        // Surrounding whitespace is tolerated.
        assert_eq!(
            heartbeat_interval_from_env(Some("  15  ".to_string())),
            Some(Duration::from_secs(15))
        );
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
