# SoRX NATS-presence producer — Implementation Plan (epic #3 slice 1, sorx side)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Checkbox steps.

**Goal:** A booting SoRX instance publishes a `SorxPresence` message (identity + capabilities) on a
NATS subject, so consumers can push-discover it. Feature-gated (`presence-nats`), default-off,
best-effort (never blocks/fails boot). This is the PRODUCER half; the operax consumer is a separate PR.

**Architecture:** New local `SorxPresence` struct (`greentic-sorx-core`); a feature-gated boot hook in
`run_start` that connects async-nats (mirroring `NatsEventSink::connect`) and publishes ONE message on
`greentic.presence.<tenant>.sorx.<sor>`, embedding `runtime_capabilities()` verbatim.

Design doc: `docs/superpowers/specs/2026-07-27-sorx-nats-presence-design.md`.

## Global Constraints

- **No `unwrap()`/`panic!()` in production.** The publish hook is best-effort: any connect/serialize/
  publish failure → `tracing::warn!` and continue; NEVER block or fail boot.
- **Default build byte-unchanged** when `presence-nats` is off OR `SORX_PRESENCE_NATS_URL` unset.
- Reuse `runtime_capabilities()` (do not duplicate capability serialization) + the `NatsEventSink`
  connect pattern (async-nats 0.46, dedicated thread + current-thread runtime). Core NATS, at-most-once.
- `#![forbid(unsafe_code)]`; English; Conventional Commits.
- `bash ci/local_check.sh` green (fmt + clippy `-D warnings` + test). The live-NATS test is
  `#[ignore]`d + feature-gated so `--all-features` doesn't require a broker.

---

### Task 1: `SorxPresence` type (`greentic-sorx-core`)

**Files:**
- Create: `crates/greentic-sorx-core/src/presence.rs`
- Modify: `crates/greentic-sorx-core/src/lib.rs` (add `pub mod presence;`)
- Test: inline `#[cfg(test)]` in `presence.rs`

**Interfaces (produced):**
```rust
pub const SORX_PRESENCE_SCHEMA: &str = "greentic.sorx.presence.v1";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SorxPresence {
    pub schema: String,
    pub instance_id: String,
    pub tenant: String,
    pub environment: String,
    pub sor: String,
    pub pack_version: String,
    pub base_url: String,
    pub reachable: bool,
    pub offers: crate::generic_runtime::RuntimeCapabilities,
    pub ts: String,
}
```

- [ ] **Step 1: Read** `crates/greentic-sorx-core/src/generic_runtime.rs` (`RuntimeCapabilities` —
  confirm it derives `Serialize, Deserialize, Clone, PartialEq, Eq`; if it lacks any needed derive,
  note it — `SorxPresence`'s derives must be satisfiable) and `business_events.rs` (the process-tag /
  `next_event_id` pattern for a stable `instance_id`, and `topic_segment` for subject sanitization).

- [ ] **Step 2: Write a failing serde round-trip test** in `presence.rs`:
```rust
#[test]
fn sorx_presence_json_round_trips() {
    let p = SorxPresence {
        schema: SORX_PRESENCE_SCHEMA.to_string(),
        instance_id: "sorx-abc-1".into(), tenant: "acme".into(), environment: "local".into(),
        sor: "landlord-tenant-sor".into(), pack_version: "0.1.0".into(),
        base_url: "https://sor.acme.example".into(), reachable: true,
        offers: RuntimeCapabilities::sorx_runtime_host(),
        ts: "2026-07-27T00:00:00Z".into(),
    };
    let bytes = serde_json::to_vec(&p).unwrap();
    let back: SorxPresence = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(p, back);
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["schema"], SORX_PRESENCE_SCHEMA);
    assert_eq!(v["reachable"], true);
}
```

- [ ] **Step 3: Run → RED, define the struct + const → GREEN.** Add `pub mod presence;` to `lib.rs`.
  Run: `cargo test -p greentic-sorx-core presence`

- [ ] **Step 4: fmt + clippy** `cargo fmt -p greentic-sorx-core -- --check` ·
  `cargo clippy -p greentic-sorx-core --all-targets -- -D warnings`

- [ ] **Step 5: Commit** — `feat(sorx-core): add SorxPresence type for NATS push-discovery`

---

### Task 2: feature-gated boot-publish hook (`greentic-sorx-cli`)

**Files:**
- Create: `crates/greentic-sorx-cli/src/presence_publish.rs` (feature-gated)
- Modify: `crates/greentic-sorx-cli/src/lib.rs` (declare the module; call the hook in `run_start`
  after `base_url` is resolved), `crates/greentic-sorx-cli/Cargo.toml` (`presence-nats` feature)
- Test: inline in `presence_publish.rs` — the `reachable`/subject/payload builder (pure) + a
  `#[cfg(feature="presence-nats")]` `#[ignore]`d live-NATS test

**Interfaces:**
- Consumes: `greentic_sorx_core::presence::{SorxPresence, SORX_PRESENCE_SCHEMA}`;
  `HttpRuntime::runtime_capabilities()`; the resolved `base_url` + `public_base_url` + tenant/sor/
  pack-version in `run_start`'s scope.

- [ ] **Step 1: `Cargo.toml` feature.** Add `presence-nats = ["dep:async-nats", "dep:tokio", ...]`
  mirroring the `events-nats` feature's dep set (read the existing `events-nats` line at ~L46 and copy
  its deps). Default-off.

- [ ] **Step 2: Read** `crates/greentic-sorx-cli/src/nats_events.rs` (`NatsEventSink::connect` — the
  thread + current-thread-runtime + `async_nats::connect(url).await` + `publish().await` pattern) and
  the `run_start` region where `base_url` is resolved + the startup banner prints (grep
  `runtime_base_url`, the `eprintln!` banner). Note the env-read pattern used for the event bridge
  (`SORX_..._NATS_URL`) to mirror for `SORX_PRESENCE_NATS_URL`.

- [ ] **Step 3: Write the pure builder + its failing test** in `presence_publish.rs`:
  `pub(crate) fn build_presence(tenant, environment, sor, pack_version, base_url, public_base_url_set: bool, offers, ts) -> SorxPresence` +
  `pub(crate) fn presence_subject(tenant: &str, sor: &str) -> String` (returns
  `greentic.presence.{san(tenant)}.sorx.{san(sor)}`, sanitizing each segment). Tests:
  `build_presence_sets_reachable_from_public_base_url` (true when set, false when not),
  `presence_subject_sanitizes_segments` (a tenant/sor with a space/`.` → `-`). Implement → GREEN.

- [ ] **Step 4: The feature-gated publish hook.**
  `#[cfg(feature = "presence-nats")] pub(crate) fn publish_presence_on_boot(presence: SorxPresence, subject: String)`:
  read `SORX_PRESENCE_NATS_URL` (return early if unset); spawn a dedicated OS thread with a
  current-thread tokio runtime (mirror `NatsEventSink::connect`); inside: `async_nats::connect(url)`,
  `serde_json::to_vec(&presence)`, `client.publish(subject, bytes).await`, `client.flush().await`;
  every failure → `tracing::warn!(...)` + return (never panic, never propagate). A
  `#[cfg(not(feature = "presence-nats"))]` no-op stub with the same signature.

- [ ] **Step 5: Wire into `run_start`.** After `base_url` is resolved (and `runtime_capabilities()`
  is available on the built `HttpRuntime`/server), call:
  ```rust
  let presence = crate::presence_publish::build_presence(
      &tenant, &environment, &sor, &pack_version, &base_url,
      public_base_url.is_some(), server.runtime_capabilities(), now_rfc3339());
  crate::presence_publish::publish_presence_on_boot(presence, crate::presence_publish::presence_subject(&tenant, &sor));
  ```
  (Adapt to the exact variable names in `run_start` — grep them: tenant/environment from
  `SorxRuntimeConfig`, sor/pack_version from the runtime pack, `public_base_url` from `ServerConfig`.
  For `now_rfc3339`, reuse whatever timestamp helper the crate already has, or `chrono::Utc::now().to_rfc3339()`
  if chrono is a dep — grep; else format from `std::time`.) Guard the whole call with
  `#[cfg(feature = "presence-nats")]` if `runtime_capabilities()` isn't otherwise reachable without it.

- [ ] **Step 6: Live-NATS test (gated + ignored)** in `presence_publish.rs`:
  `#[cfg(feature = "presence-nats")] #[tokio::test] #[ignore]` — connect a test subscriber to a local
  `nats-server`, publish via the hook, assert the subscriber receives + decodes a `SorxPresence` with
  the right subject/schema. Mirror `crates/greentic-sorx-cli/tests/nats_events_integration.rs`'s setup
  (grep it). `#[ignore]` so `--all-features` (local_check) skips it without a broker.

- [ ] **Step 7: Verify.**
  - `cargo test -p greentic-sorx-core -p greentic-sorx-cli` (default features; the pure tests pass, no
    broker needed; the ignored live test is skipped)
  - `cargo build -p greentic-sorx-cli --features presence-nats` (feature compiles)
  - `cargo fmt --all -- --check` · `cargo clippy --all-targets -- -D warnings` (default) AND
    `cargo clippy --all-targets --features presence-nats -- -D warnings` (feature-on)

- [ ] **Step 8: Commit** — `feat(sorx): publish SorxPresence on boot for NATS push-discovery (presence-nats, default-off)`

---

## Final verification (before PR)

- [ ] `bash ci/local_check.sh` green (default features; presence-nats off → byte-unchanged default). If
  the sorx `perf` job is pre-existing-red (per epic memory), document it, don't hide it.
- [ ] Confirm default boot path is unchanged when the feature is off / env unset (the hook is a no-op).
- [ ] PR → `greentic-sorx` `research`. Title `feat(sorx): NATS-presence push-discovery producer (epic #3, presence-nats)`.
  Body: reuses NatsEventSink + runtime_capabilities; feature-gated default-off; `reachable` flag for
  loopback; boot-publish only (heartbeat = follow-up); operax presence-subscriber consumer is a
  separate PR (slice 1).

## Out of scope (this PR)

- Heartbeat / periodic re-publish; operax consumer (separate PR); `HttpSorxClient` directory-fallback;
  promoting `SorxPresence` to `greentic-types`.
