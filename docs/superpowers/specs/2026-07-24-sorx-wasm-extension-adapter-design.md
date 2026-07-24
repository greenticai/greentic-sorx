# SoRX WASM extension adapter — design (Sub-B)

_Date: 2026-07-24 · Repo: `greentic-sorx` · Branch: `feat/sorx-wasm-extension-adapter` (off `research`)_

## Context

SoRLa/SoRX productionization epic, item **#6-B phase 2**, **Sub-B**. Phase-1 (merged) made the
extension-dispatch seam live and shipped a native in-process audit observer. **Sub-A** (merged to
`greentic-designer-extensions` `develop` @ `c28e957`) added a `greentic:extension-sorx@0.1.0` WIT
world + `ExtensionRuntime::{control,observe}` dispatch to `greentic-ext-runtime`.

**Sub-B** makes SoRX actually run WASM extension packs: a `WasmExtensionRuntime` that implements the
existing `RuntimeExtensionAdapter` trait by calling Sub-A's `ExtensionRuntime::{control,observe}`.
Architecture decision (made with the user): reuse `greentic-ext-runtime` via a git dependency.

**Sub-C** (later) builds a guest `wasm32-wasip2` component + a true end-to-end execution test.
Sub-B ships the adapter, wiring, config, and adapter-level tests only — **no guest WASM runs here**.

## Goal (Sub-B)

1. `greentic-sorx` depends on `greentic-ext-runtime` (git, `rev = c28e957`), behind a cargo feature.
2. A `WasmExtensionRuntime` (`greentic-sorx-cli`) implements `RuntimeExtensionAdapter`, mapping
   `binding.pack_ref` → an ext-runtime `ExtensionId` and dispatching `control`/`observe` with the
   serde-JSON of SoRX's binding/request/response/event, parsing back a `ControlDecision`.
3. `bind_runtime_extensions` (phase-1) registers the WASM adapter for declared wasm bindings,
   alongside the native audit observer, from a configured extension directory.
4. When the feature is off (default), behavior is identical to today (native-only) — no wasmtime
   dependency compiled in.

## Non-goals (Sub-B)

- No guest WASM component and no real-execution test (Sub-C).
- No new WIT / no change to `greentic-ext-runtime` (Sub-A is frozen at `c28e957`).
- No hot-reload/watcher, no instance pooling tuning (use ext-runtime defaults).
- No signing infrastructure — reuse ext-runtime's verify chain + its `dev-allow-unsigned` escape.

## Architecture

### Dependency (feature-gated)

Add to `greentic-sorx-cli/Cargo.toml`, behind a `wasm-extensions` feature:

```toml
[features]
wasm-extensions = ["dep:greentic-ext-runtime"]

[dependencies]
greentic-ext-runtime = { git = "https://github.com/greentic-biz/greentic-designer-extensions", rev = "c28e957", optional = true }
```

Rationale: the git dep pulls wasmtime 43 + cranelift + wasi + `greentic-extension-sdk-contract`
(`=1.2.1-research`) — a heavy addition to the operator runtime. Feature-gating keeps the default
`gtc`/sorx binary light; wasm extensions are opt-in. The `WasmExtensionRuntime` type and all its
wiring are `#[cfg(feature = "wasm-extensions")]`. **The adapter lives in `greentic-sorx-cli`, not
`greentic-sorx-core`** — core stays wasmtime-free; a cli type implementing the core
`RuntimeExtensionAdapter` trait is fine.

> Confirm at implementation time whether `rev = c28e957` resolves cleanly (its transitive
> `=1.2.1-research` sdk-contract pin must not conflict with anything already in the sorx graph —
> today sorx has no wasm/ext deps, so no conflict is expected). If a maintainer later cuts a
> `v1.2.X-research` tag containing `c28e957`, prefer pinning `tag = "..."` over `rev`.

### `WasmExtensionRuntime` (`greentic-sorx-cli`, `#[cfg(feature = "wasm-extensions")]`)

```rust
struct WasmExtensionRuntime { runtime: Arc<ExtensionRuntime> }

impl RuntimeExtensionAdapter for WasmExtensionRuntime {
    fn control(&self, hook, binding, request, response) -> SorxResult<ControlDecision> {
        // binding_json = serde_json::to_string(binding)
        // request_json = serde_json::to_string(request)  (request is &Value)
        // response_json = response.map(to_string)
        // let out = self.runtime.control(&binding.pack_ref, hook, &binding_json, &request_json, response_json.as_deref())
        //             .map_err(|e| SorxError::new("wasm_extension_control_failed", e.to_string()))?;
        // serde_json::from_str::<ControlDecision>(&out).map_err(...)
    }
    fn observe(&self, subscription, binding, event) -> SorxResult<()> {
        // event_json = serde_json::to_string(event); binding_json as above
        // self.runtime.observe(&binding.pack_ref, subscription, &binding_json, &event_json).map_err(...)
    }
}
```

`binding.pack_ref` **is** the ext-runtime `ExtensionId` — the operator installs a signed extension
dir whose describe id equals the `pack_ref` declared in `RuntimeConfig.extensions`. No separate
mapping table.

### Construction + registration (`from_pack_with_runtime_config` / `bind_runtime_extensions`)

- Build one `Arc<ExtensionRuntime>` at runtime construction, from a `RuntimeConfig`/`DiscoveryPaths`
  pointing at the SoRX extension directory (see Config). Registration of installed extension dirs
  uses ext-runtime's existing `ExtensionRuntime::new` (discovers from the configured root) — no new
  discovery kind is added on the ext-runtime side (Sub-A non-goal preserved); sorx points
  `DiscoveryPaths` at its own root.
- Extend `bind_runtime_extensions` (phase-1): after registering the native audit adapter, for each
  declared binding whose `pack_ref` is **not** `NATIVE_AUDIT_PACK_REF`, register the shared
  `Arc<WasmExtensionRuntime>` under that `pack_ref`. If that extension is not actually loaded, the
  adapter's `control`/`observe` returns `RuntimeError::NotFound` → `SorxError`, honored by the
  binding's `fail_mode` (Closed → deny; Open → skip). No pre-flight "is it loaded" check needed.
- When the `wasm-extensions` feature is off, `bind_runtime_extensions` compiles without the WASM
  branch — native-only, identical to phase-1.

### Config

Add a minimal, optional extension-runtime config. Recommended: an env-driven root consistent with
phase-1's `SORX_STATE_DIR` style and ext-runtime's own conventions —
`SORX_EXTENSIONS_DIR` (defaults to `~/.greentic/extensions/sorx/`), plus ext-runtime's existing
`GREENTIC_EXT_ALLOW_UNSIGNED` (+ the `dev-allow-unsigned` feature) for local unsigned dev. If a
structured field is preferred, add `SorxRuntimeConfig.extensions: Option<ExtensionRuntimeConfig>`
(`{ dir: Option<String> }`) — decide at plan time based on how other runtime paths are configured;
keep it to one knob. No secrets, no per-tenant config in Sub-B.

## Async / blocking note (for Sub-C / production, not tested here)

`ExtensionRuntime::{control,observe}` run the WIT call on a **sync** wasmtime store and can block.
SoRX's `ControlHook`/`ObserverHook` are already sync, so the adapter body is a straight sync call.
The hazard is only if SoRX's HTTP layer drives `invoke` on an async task — then the sync WASM call
blocks the executor and the caller should `spawn_blocking`. Sub-B runs no real WASM (adapter tests
hit the `NotFound`/serialization paths only), so this is latent; flag it for Sub-C + production
hardening, do not solve it here.

## Testing (Sub-B, adapter-level, `#[cfg(feature = "wasm-extensions")]`)

- `WasmExtensionRuntime::control`/`observe` against an `ExtensionRuntime` with **no** loaded
  extension (e.g. `ExtensionRuntime::for_test()` or a runtime over an empty tempdir) → the
  underlying `RuntimeError::NotFound` maps to a `SorxError` (adapter returns `Err`). Proves the
  pack_ref→ext_id call + error mapping is wired.
- Serialization: `control` given a real `RuntimeExtensionBinding` + request `Value` serializes them
  without panicking (assert the call reaches `ExtensionRuntime` and returns the NotFound error, not
  a serialization error).
- Wiring parity: with the feature on but no wasm bindings declared, `bind_runtime_extensions`
  behaves as phase-1 (native audit only; no panic). With a wasm `pack_ref` declared,
  `bind_runtime_extensions` registers the WASM adapter under that pack_ref (assert the registry
  resolves it — e.g. via a NotFound dispatch through `BoundControlHook`).
- Real control/observe **execution** against a compiled guest is Sub-C.

## Files touched

- `greentic-sorx-cli/Cargo.toml` — `wasm-extensions` feature + optional git dep.
- `greentic-sorx-cli/src/` — new `WasmExtensionRuntime` module (`#[cfg(feature)]`); extend
  `bind_runtime_extensions` in `http_runtime.rs`; build the `ExtensionRuntime` in
  `from_pack_with_runtime_config`.
- `greentic-sorx-core` — untouched (stays wasmtime-free).
- Docs: extend `docs/extensions.md` (phase-1) with the WASM-extension section + the feature flag.

## Global constraints (from this repo)

- Rust 1.95.0; `#![forbid(unsafe_code)]` norm; no `unwrap()`/`panic!()` in production (tests may
  unwrap); `SorxResult`/`SorxError`.
- English only; Conventional Commits; sorx allows the Claude co-author trailer (unlike the
  designer-extensions repo).
- `bash ci/local_check.sh` before done. The sorx CI `perf` job fails environmentally (non-required).
- Gate the wasm feature build explicitly: run `cargo test -p greentic-sorx --features wasm-extensions`
  AND the default `cargo test -p greentic-sorx` (feature off) — both must pass. `--all-features` in
  the repo's local_check must resolve the git dep (network); if CI cannot fetch the git dep, the
  feature must remain off in the default gate and the wasm gate run separately.
