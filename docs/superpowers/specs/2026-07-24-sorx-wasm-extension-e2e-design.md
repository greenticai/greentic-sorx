# SoRX WASM extension end-to-end test — design (Sub-C)

_Date: 2026-07-24 · Repo: `greentic-sorx` · Branch: `feat/sorx-wasm-extension-e2e` (off `research`)_

## Context

SoRLa/SoRX epic, item **#6-B phase 2**, **Sub-C** (final piece). Sub-A (merged, `greentic-ext-runtime`
@ `c28e957`) added the `greentic:extension-sorx` WIT world + `ExtensionRuntime::{control,observe}`.
Sub-B (merged to `greentic-sorx` `research`) added the feature-gated `WasmExtensionRuntime` adapter +
wiring, tested only at the adapter level (unloaded-extension → error) — **no real WASM ran**.

**Sub-C proves real execution**: a `wasm32-wasip2` guest component implementing the world, loaded
dev-unsigned, invoked through `WasmExtensionRuntime`, asserting the guest's actual `ControlDecision`
comes back. This validates the two things Sub-A/B could not: the `get_export_index` interface-id
string resolves against a compiled component, and real JSON payloads round-trip.

## Goal (Sub-C)

1. A guest crate (a `wasm32-wasip2` component) exporting `control`/`observe` for the
   `greentic:extension-sorx` world, in the sorx tree as a test fixture.
2. A feature-gated end-to-end test: build the guest, assemble a signed-unsigned extension dir,
   load it via `greentic-ext-runtime` (dev-unsigned), dispatch `control`/`observe` through
   `WasmExtensionRuntime`, and assert the guest's decision (e.g. a `deny` for a marker input, an
   `allow` otherwise; an `observe` that succeeds).
3. Confirm + document the async-safety invariant for WASM dispatch (no new hardening code needed —
   see below).

## Key finding: `spawn_blocking` hardening is already satisfied (no code change)

The Sub-B review flagged that sync WASM calls could block an async executor. Investigation shows the
concern does **not** apply to SoRX as built:

- SoRX's HTTP server is **not** tokio-async: `serve()` uses `std::net::TcpListener` and
  `std::thread::spawn` per connection (`http_runtime.rs:324-337`); `handle_request → SorxRuntime::invoke
  → BoundControlHook/ObserverHook → WasmExtensionRuntime::{control,observe}` all run synchronously on a
  dedicated OS thread. There is no async reactor in that stack to stall.
- The one async entry point — the NATS event-bridge — **already** wraps `runtime.invoke` in
  `tokio::task::spawn_blocking` (`event_bridge_invoker.rs:104-108`, with a comment stating exactly this
  rationale).

So Sub-C adds **no** `spawn_blocking` code. It (a) documents the invariant — *"any async caller of
`SorxRuntime::invoke` or `WasmExtensionRuntime::{control,observe}` must dispatch on
`tokio::task::spawn_blocking`, as `event_bridge_invoker.rs` does"* — in `docs/extensions.md`, and (b)
optionally adds a short doc-comment to `WasmExtensionRuntime` noting its methods are blocking.

## Non-goals

- No real signing / store distribution — the fixture loads dev-unsigned.
- No new discovery kind in ext-runtime (Sub-A frozen); point `DiscoveryPaths`/`register_loaded_from_dir`
  at the fixture dir.
- No committed `.wasm` binary — the guest is built from source by the test (or a helper script), gated
  so default CI (which lacks `cargo-component`) skips it.
- No change to the merge-gate CI (`ci.yml` stays credential-free, feature-off).

## Architecture

### Guest crate (`tests/fixtures/sorx-e2e-guest/`, NOT a workspace member)

A `cdylib` component built with `cargo-component`, mirroring `telco-x-designer-ext`:

- `Cargo.toml`: `crate-type = ["cdylib"]`; `wit-bindgen-rt = "0.41"`; `[package.metadata.component]`
  `package = "greentic:sorx-e2e"`, target `path = "wit"`, target deps pointing at vendored
  `greentic:extension-sorx` + `greentic:extension-host`. Exclude it from the sorx workspace
  (`[workspace] exclude = ["tests/fixtures/sorx-e2e-guest"]` or place outside `members` globs) so the
  normal `cargo build` never tries to build it for the host target.
- `wit/`: the world file (`world sorx-runtime-extension` exporting `control`/`observe`, importing
  `greentic:extension-host/logging`) + vendored `wit/deps/greentic/extension-sorx/…` and
  `wit/deps/greentic/extension-host/…` (copied verbatim from the pinned ext-runtime checkout — the
  `logging` interface is byte-identical to the telco-x copy).
- `src/lib.rs`: `mod bindings;` (cargo-component-generated, gitignored) + `impl control::Guest` and
  `impl observe::Guest` for a `Component` struct + `bindings::export!(Component with_types_in bindings)`.
  Behavior (deterministic, assertable):
  - `control(hook, binding_json, request_json, response_json)`: parse `request_json`; if it contains a
    marker (e.g. `"deny": true`) return `Ok(r#"{"action":"deny","reason":"e2e guest denied"}"#)`, else
    `Ok(r#"{"action":"allow"}"#)`. This lets the test assert BOTH a passthrough-allow and a real deny.
  - `observe(subscription, binding_json, event_json)`: `logging::log(Info, "sorx-e2e-guest", …)` then
    `Ok(())`.

### Fixture dir + `describe.json`

The test assembles a temp dir `<root>/<id>/` containing:
- `extension.wasm` — the built guest binary (copied from `target/wasm32-wasip2/release/…`).
- `describe.json` — the minimal schema-valid document (verified against
  `greentic-extension-sdk-contract 1.2.1-research`): `apiVersion: "greentic.ai/v2"`, `kind:
  "ProviderExtension"`, `metadata.id = "greentic.sorx.e2e-guest"`, `runtime.components` with ONE entry
  whose `gtpack.file = "extension.wasm"`, both `sha256` fields = 64 zero-hex placeholders (never
  checked on the unsigned path), `world:
  "greentic:extension-sorx/sorx-runtime-extension@0.1.0"`. (`describe.json` JSON-Schema validation runs
  unconditionally, so it must be well-formed; `manifest.json` is not needed on the unsigned path.)

The binding's `pack_ref` in the test's `RuntimeConfig.extensions` == `metadata.id`
(`greentic.sorx.e2e-guest`) == the ext id `WasmExtensionRuntime` passes to `ExtensionRuntime::control`.

### The e2e test (`greentic-sorx-cli`, feature `wasm-extensions-dev-unsigned`)

Add a cargo feature:
```toml
wasm-extensions-dev-unsigned = ["wasm-extensions", "greentic-ext-runtime/dev-allow-unsigned"]
```
Test flow (a single `#[test]`, only compiled under that feature — default CI skips it):
1. Build the guest: `cargo component build --release --target wasm32-wasip2` in the fixture crate
   (shell out via `std::process::Command`, into a temp/ignored target). If `cargo-component` is
   unavailable, `panic!`/skip with a clear message — the feature is opt-in, so this only runs when the
   toolchain is present.
2. Assemble the fixture dir (`describe.json` + `extension.wasm`) under a `tempfile::TempDir`.
3. `SAFETY`: set `GREENTIC_EXT_ALLOW_UNSIGNED=1` for the process; build
   `ExtensionRuntime::new(RuntimeConfig::from_paths(DiscoveryPaths::new(<root>)))` (or
   `register_loaded_from_dir(<dir>)`), wrap in `WasmExtensionRuntime::new(Arc::new(rt))`.
4. Call `adapter.control("pre_call", &binding("greentic.sorx.e2e-guest"), &json!({"deny": true}), None)`
   → assert `ControlDecision.action == Deny` and reason present. Call again with `json!({})` → assert
   `Allow`. Call `adapter.observe("post_call", &binding, &json!({...}))` → assert `Ok(())`.
   These assertions prove: the interface-id string resolves, the guest actually ran, and real
   `ControlDecision` JSON round-trips.

### Build orchestration

Add `ci/build-sorx-e2e-guest.sh` (mirroring ext-runtime's `ci/build-ac-ext.sh`) that runs the
`cargo component build`, so the guest can be built independently of the test if preferred; the test may
either shell out itself or expect the script to have run (decide at plan time — prefer the test
shelling out so it is self-contained, guarded by the feature). Document that the e2e test needs
`cargo-component` + the `wasm32-wasip2` target, and is opt-in via `--features wasm-extensions-dev-unsigned`.

## Files touched

- `greentic-sorx` — new `tests/fixtures/sorx-e2e-guest/` (Cargo.toml, wit/, src/lib.rs, .gitignore for
  `bindings.rs`); `[workspace] exclude` entry; new `wasm-extensions-dev-unsigned` feature in
  `greentic-sorx-cli/Cargo.toml`; the e2e test (in `wasm_extensions.rs` or a `tests/` integration file,
  feature-gated); `ci/build-sorx-e2e-guest.sh`; `docs/extensions.md` (e2e + async-safety invariant).
- No change to `greentic-sorx-core`, the Sub-B adapter/wiring, or `ci.yml`.

## Global constraints (from this repo)

- Rust 1.95; `#![forbid(unsafe_code)]` norm; no `unwrap()`/`panic!()` in production (test code may
  unwrap); `SorxResult`/`SorxError`.
- English only; Conventional Commits; sorx allows the Claude co-author trailer.
- The default `cargo test -p greentic-sorx` and the merge gate stay unchanged (the e2e test is only
  compiled under `wasm-extensions-dev-unsigned`, which needs `cargo-component` + network for the git
  dep + the wasm target). Run the e2e locally / in a dedicated job with `GREENTIC_EXT_ALLOW_UNSIGNED=1`.
- Do NOT enable `dev-allow-unsigned` in any production/default feature — it is test-only.
