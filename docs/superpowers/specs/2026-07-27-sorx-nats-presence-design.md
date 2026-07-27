# SoRX NATS-presence self-register / push-discovery — design (epic #3, slice 1)

_Date: 2026-07-27 · Repos: `greentic-sorx` (producer) + `greentic-operax` (consumer) · sorx branch `feat/sorx-presence` off `research`_

## Context

Epic requirement #3 (discover/interact/subscribe deployed SoRLa instances): interact ✅, subscribe/
receive ✅ (operax PR#8), publish ✅. Discover is **PULL only** — a consumer must already know a
SoRX instance's `base_url` (operax's `HttpSorxClient` takes a static `--sorx-url`). This adds
**push-discovery**: a booting SoRX instance announces itself + its capabilities on a NATS subject; a
consumer subscribes to build a live directory of instances it didn't already know.

Substrate = **reuse the existing NATS infra** (the `events-nats` sink already connects async-nats
0.46 and publishes `<prefix>.<tenant>.<topic>`). NO new registry/directory service. Chosen with the
user; net-new-both-sides but light (mirrors shipped patterns).

## Non-goals (slice 1)

- Heartbeat / periodic re-publish (boot-time publish only). Follow-up.
- Wiring the discovered address into `HttpSorxClient` (operax keeps `--sorx-url`; the subscriber only
  builds + logs a directory). Follow-up.
- Durability/JetStream/request-reply (core NATS, at-most-once — matches the existing sink).
- Promoting `SorxPresence` to `greentic-types` (define it LOCAL in `greentic-sorx-core` to avoid the
  greentic-types release-train gate; promote later once the shape stabilizes).

## Design — producer (`greentic-sorx`)

### `SorxPresence` (new, local to `greentic-sorx-core`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SorxPresence {
    pub schema: String,            // const "greentic.sorx.presence.v1"
    pub instance_id: String,       // stable per-process (reuse business_events process-tag pattern)
    pub tenant: String,
    pub environment: String,
    pub sor: String,               // pack name
    pub pack_version: String,
    pub base_url: String,          // runtime_base_url() result
    pub reachable: bool,           // true iff public_base_url was configured (else base_url is a
                                   // loopback/local socket a remote consumer can't reach)
    pub offers: RuntimeCapabilities, // reuse runtime_capabilities() verbatim (same as GET /admin/v1/capabilities)
    pub ts: String,                // RFC3339 publish timestamp
}
```
Serialized JSON. `reachable=false` lets a consumer skip loopback-only instances (see blocker below).

### Publish hook (`greentic-sorx-cli` `run_start`)

Gated by a new Cargo feature `presence-nats` (default-off; same dep set as `events-nats`:
async-nats/tokio) AND activated at runtime only when `SORX_PRESENCE_NATS_URL` is set (mirrors how the
event bridge reads its NATS URL from env). At boot, after `base_url` is resolved (the point where the
startup banner prints), publish ONE `SorxPresence` message:

- subject: `greentic.presence.{tenant}.sorx.{sor}` (its OWN `greentic.presence` prefix, NOT the
  `greentic.events` business-event prefix, so a consumer can `subscribe("greentic.presence.>")`
  without business-event traffic). Sanitize `{tenant}`/`{sor}` segments with the same rule
  `business_events::topic_segment` uses (vendor a tiny local sanitizer if it's private; keep
  `[A-Za-z0-9_-]`, else `-`).
- connection: mirror `NatsEventSink::connect` (a dedicated OS thread + current-thread tokio runtime,
  `async_nats::connect(url).await`, one `publish(subject, bytes).await`, then done — no long-lived
  drain loop needed for a single boot publish). Best-effort: any connect/publish failure logs a warn
  and NEVER blocks or fails boot (the server still starts + serves; presence is advisory).
- payload: `serde_json::to_vec(&SorxPresence { ... offers: self.runtime_capabilities(), ... })`.

Keep the whole hook behind `#[cfg(feature = "presence-nats")]`; a no-op stub otherwise.

### Blocker handling — unreachable loopback

`runtime_base_url()` falls back to `http://{local_addr}` (often loopback/container-internal) when
`public_base_url` is unset. Publishing that as a reachable address would mislead a remote consumer.
Set `reachable: false` when `public_base_url` was absent/empty (and `true` when it was configured);
still publish (a consumer can choose to ignore `reachable=false`, and a local/same-host consumer may
still use it). Do NOT refuse to publish — that would make presence silently absent; the flag is the
honest signal.

## Design — consumer (`greentic-operax`, separate follow-on PR)

Feature-gated (extend the existing `events` feature or a new `presence`). A
`operax presence subscribe --nats-url <url> [--tenant <t>]` subcommand (mirror `events subscribe`):

- `async_nats::connect(nats_url)`, `subscribe("greentic.presence.{tenant}.>")` (or
  `greentic.presence.>` when no tenant filter) — mirror `business_events.rs::run_subscriber`.
- For each message: `serde_json::from_slice::<SorxPresence>` (skip+log malformed). Maintain an
  in-memory directory `HashMap<instance_id, DirectoryEntry { presence: SorxPresence, last_seen: Instant }>`.
  On each message: upsert + log the current directory (instance_id → sor/version/base_url/reachable).
- Soft-state eviction sweep: a background tick drops entries older than a TTL (e.g. 2.5× a nominal
  heartbeat; since slice-1 has no heartbeat, use a generous default like 5 min and document that
  eviction is meaningful only once the producer heartbeat lands).
- Pure, unit-testable core: `apply_presence(&mut directory, SorxPresence, now)` + `evict_stale(&mut
  directory, now, ttl)` — tested without NATS. The NATS connect/subscribe loop is thin glue.

`SorxPresence` shape is duplicated operax-side as a local `#[derive(Deserialize)]` mirror (operax
already mirrors sorx wire shapes; it does not dep greentic-sorx-core). Keep the JSON field set
identical.

## Testing

- **sorx**: `SorxPresence` serde round-trip (unit); `reachable` reflects public_base_url presence
  (unit on the builder helper); a `#[cfg(feature="presence-nats")]` `#[ignore]`d live-NATS test
  (publish → a test subscriber receives + decodes) mirroring `nats_events_integration.rs`. Default
  build/CI unchanged (feature off).
- **operax**: `apply_presence` upserts + updates `last_seen`; `evict_stale` drops old entries, keeps
  fresh; malformed slice skipped. Pure, no live NATS. A gated/ignored live round-trip optional.

## Files touched

- `greentic-sorx`: `crates/greentic-sorx-core/src/presence.rs` (new — `SorxPresence` + const +
  `reachable` helper); `crates/greentic-sorx-cli/src/presence_publish.rs` (new, feature-gated — the
  connect+publish hook); `crates/greentic-sorx-cli/src/lib.rs` (`run_start` calls the hook after
  base_url; feature wiring); `crates/greentic-sorx-cli/Cargo.toml` (`presence-nats` feature).
- `greentic-operax` (follow-on PR): `crates/operax-cli/src/presence.rs` (new — directory + pure
  apply/evict + subscriber loop); CLI `presence subscribe` subcommand; feature gate.

## Global constraints

- Rust edition/toolchain per repo; `#![forbid(unsafe_code)]`; **no `unwrap()`/`panic!()` in
  production** — the publish hook is best-effort (warn, never fail boot); the subscriber skips
  malformed messages.
- Default build unchanged when the feature is off / env unset (no new default deps).
- async-nats 0.46 (match `events-nats`); core NATS at-most-once.
- English; Conventional Commits; sorx `bash ci/local_check.sh` (fmt/clippy `-D warnings`/test) green;
  the live-NATS test `#[ignore]`d so `--all-features` doesn't require a broker.
- Reuse `runtime_capabilities()` + the NatsEventSink connect pattern; do NOT duplicate capability
  serialization.

See epic memory `sorla-sorx-productionization-epic` (#3) for the full grounding + why NATS-presence
over a new registry service.
