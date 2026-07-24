# SoRX WASM Extension Adapter — Implementation Plan (Sub-B)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `greentic-sorx` run WASM extension packs: a feature-gated `WasmExtensionRuntime` (in `greentic-sorx-cli`) implementing the core `RuntimeExtensionAdapter` trait by calling `greentic-ext-runtime`'s `ExtensionRuntime::{control,observe}` (Sub-A, git-pinned at `rev=c28e957`), wired into `bind_runtime_extensions` alongside the phase-1 native audit observer.

**Architecture:** git-dep on `greentic-ext-runtime` behind a `wasm-extensions` cargo feature (heavy wasmtime tree stays opt-in; default build unchanged). Adapter in `sorx-cli` so `greentic-sorx-core` stays wasmtime-free. `binding.pack_ref` == ext-runtime `ExtensionId`. Real WASM execution against a compiled guest is Sub-C — this ships adapter + wiring + adapter-level tests only.

**Tech Stack:** Rust 1.95, `greentic-ext-runtime` (git, wasmtime 43), `serde_json`.

## Global Constraints

- Rust 1.95.0; `#![forbid(unsafe_code)]`; no `unwrap()`/`panic!()` in production (tests may unwrap); `SorxResult`/`SorxError`.
- English only; Conventional Commits; sorx **allows** the `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer.
- `greentic-sorx-core` must NOT gain a wasmtime/ext-runtime dependency — the adapter lives in `greentic-sorx-cli`, `#[cfg(feature = "wasm-extensions")]`.
- Default build/behavior unchanged when the feature is off (phase-1 native-only).
- Gate BOTH: `cargo test -p greentic-sorx` (feature off) AND `cargo test -p greentic-sorx --features wasm-extensions` (feature on) must pass; `cargo fmt --all -- --check`; `cargo clippy -p greentic-sorx --all-targets -- -D warnings` (and again with `--features wasm-extensions`). Do NOT run `--features foundationdb`. The sorx CI `perf` job fails environmentally (non-required).
- The git dep needs network to fetch `github.com/greentic-biz/greentic-designer-extensions` at `rev=c28e957`; its transitive `greentic-extension-sdk-contract =1.2.1-research` must resolve.

**Reference signatures (consume, do not redefine):**

```rust
// greentic-sorx-core (already present)
pub trait RuntimeExtensionAdapter: std::fmt::Debug + Send + Sync {
    fn control(&self, hook: &str, binding: &RuntimeExtensionBinding, request: &Value, response: Option<&Value>) -> SorxResult<ControlDecision> { Ok(ControlDecision::allow()) }
    fn observe(&self, subscription: &str, binding: &RuntimeExtensionBinding, event: &Value) -> SorxResult<()> { Ok(()) }
}
pub struct RuntimeExtensionBinding { pub id: String, pub contract: String, pub pack_ref: String, pub fail_mode: ExtensionFailMode }
pub struct ControlDecision { pub action: ControlDecisionAction, pub reason: Option<String>, pub patch: Option<Value> } // Serialize+Deserialize
pub struct RuntimeExtensionRegistry; // ::new(); .with_adapter(pack_ref: impl Into<String>, Arc<dyn RuntimeExtensionAdapter>) -> Self
pub const NATIVE_AUDIT_PACK_REF: &str; // "greentic.sorx.audit.v1"
pub struct RuntimeExtensions { pub control: RuntimeControlExtensions /*.hooks: BTreeMap<String,Vec<Binding>>*/, pub observer: RuntimeObserverExtensions /*.subscriptions: BTreeMap<String,Vec<Binding>>*/, pub admin }
// greentic-ext-runtime (git, Sub-A)
pub struct ExtensionRuntime; // ::new(RuntimeConfig) -> Result<Self, RuntimeError>; ::for_test() -> Self
//   .control(ext_id: &str, hook: &str, binding_json: &str, request_json: &str, response_json: Option<&str>) -> Result<String, RuntimeError>
//   .observe(ext_id: &str, subscription: &str, binding_json: &str, event_json: &str) -> Result<(), RuntimeError>
pub struct RuntimeConfig; // ::from_paths(DiscoveryPaths) -> Self
pub struct DiscoveryPaths; // ::new(user: PathBuf) -> Self
pub enum RuntimeError; // Display
```

**Current `bind_runtime_extensions` (phase-1, `http_runtime.rs`) — extend it:**
```rust
fn bind_runtime_extensions(runtime, extensions: Option<&RuntimeExtensions>, audit_sink, pack, version) -> SorxRuntime {
    let Some(extensions) = extensions.filter(|ext| !ext.is_empty()) else { return runtime };
    let registry = native_extension_registry(audit_sink, pack, version);
    let control = Arc::new(BoundControlHook::new(extensions.clone(), registry.clone()));
    let observer = Arc::new(BoundObserverHook::new(extensions.clone(), registry));
    runtime.with_control_hook(control).with_observer_hook(observer, true)
}
```

---

### Task 1: Cargo feature + git dependency

**Files:**
- Modify: `crates/greentic-sorx-cli/Cargo.toml`

**Interfaces:**
- Produces: cargo feature `wasm-extensions` enabling the optional `greentic-ext-runtime` dep.

- [ ] **Step 1: Add the feature + optional dep**

In `crates/greentic-sorx-cli/Cargo.toml`, under `[features]` add:
```toml
wasm-extensions = ["dep:greentic-ext-runtime"]
```
Under `[dependencies]` add:
```toml
greentic-ext-runtime = { git = "https://github.com/greentic-biz/greentic-designer-extensions", rev = "c28e9577dbb3227fe38c20e948b2d8b036fdc26e", optional = true }
```
> Use the full 40-char sha `c28e9577dbb3227fe38c20e948b2d8b036fdc26e` (the PR #115 merge commit on `develop`). Verify the exact remote URL against how other Greentic git deps in the workspace are written (org may be `greentic-biz` or `greenticai`) — match a working example if one exists.

- [ ] **Step 2: Verify resolution (feature on)**

Run: `cargo update -p greentic-ext-runtime 2>&1 | tail -20` then `cargo check -p greentic-sorx --features wasm-extensions 2>&1 | tail -20`
Expected: the git dep fetches and resolves (wasmtime 43 + `greentic-extension-sdk-contract =1.2.1-research` compile). If resolution fails on an sdk-contract version conflict, STOP and report — do not force-edit unrelated pins. Cold build is long; run in background and WAIT.
Run: `cargo check -p greentic-sorx 2>&1 | tail -5` — default (feature off) must still resolve WITHOUT fetching the git dep.

- [ ] **Step 3: Commit**

```bash
git add crates/greentic-sorx-cli/Cargo.toml Cargo.lock
git commit -m "build(sorx-cli): add wasm-extensions feature + greentic-ext-runtime git dep

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `WasmExtensionRuntime` adapter

**Files:**
- Create: `crates/greentic-sorx-cli/src/wasm_extensions.rs`
- Modify: `crates/greentic-sorx-cli/src/` module root (`lib.rs` or `main.rs` / wherever modules are declared) — add `#[cfg(feature = "wasm-extensions")] mod wasm_extensions;`

**Interfaces:**
- Consumes: `greentic_ext_runtime::{ExtensionRuntime, RuntimeError}`; `greentic_sorx_core::{RuntimeExtensionAdapter, RuntimeExtensionBinding, ControlDecision, SorxError, SorxResult}`.
- Produces: `pub struct WasmExtensionRuntime` with `pub fn new(runtime: Arc<ExtensionRuntime>) -> Self`, `impl RuntimeExtensionAdapter for WasmExtensionRuntime`.

- [ ] **Step 1: Write the failing tests**

Create `crates/greentic-sorx-cli/src/wasm_extensions.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use greentic_sorx_core::{ExtensionFailMode, RuntimeExtensionBinding, RuntimeExtensionAdapter};
    use serde_json::json;
    use std::sync::Arc;

    fn binding(pack_ref: &str) -> RuntimeExtensionBinding {
        RuntimeExtensionBinding {
            id: "x".into(), contract: "greentic.cap.extension.control.v1".into(),
            pack_ref: pack_ref.into(), fail_mode: ExtensionFailMode::Closed,
        }
    }

    #[test]
    fn control_on_unloaded_extension_errors() {
        let rt = Arc::new(greentic_ext_runtime::ExtensionRuntime::for_test());
        let adapter = WasmExtensionRuntime::new(rt);
        let err = adapter
            .control("pre_call", &binding("does.not.exist"), &json!({}), None)
            .unwrap_err();
        // maps ext-runtime NotFound to a SorxError (does not panic, does not silently allow)
        assert!(err.to_string().to_lowercase().contains("not") || !err.code.is_empty());
    }

    #[test]
    fn observe_on_unloaded_extension_errors() {
        let rt = Arc::new(greentic_ext_runtime::ExtensionRuntime::for_test());
        let adapter = WasmExtensionRuntime::new(rt);
        assert!(adapter
            .observe("post_call", &binding("does.not.exist"), &json!({}))
            .is_err());
    }
}
```
> Confirm the `SorxError` public shape (does it expose `.code`? `.to_string()`?) by opening its definition in `greentic-sorx-core`; adjust the assertion to a field/Display that exists. Confirm `ExtensionRuntime::for_test()` is public in the git dep (it is per Sub-A's runtime.rs); if not, build a runtime over an empty tempdir via `ExtensionRuntime::new(RuntimeConfig::from_paths(DiscoveryPaths::new(tempdir.path().to_path_buf())))`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p greentic-sorx --features wasm-extensions wasm_extensions 2>&1 | tail -20`
Expected: FAIL to compile — `WasmExtensionRuntime` not defined.

- [ ] **Step 3: Implement**

Above the test module in `wasm_extensions.rs`:
```rust
use std::sync::Arc;

use greentic_ext_runtime::ExtensionRuntime;
use greentic_sorx_core::{
    ControlDecision, RuntimeExtensionAdapter, RuntimeExtensionBinding, SorxError, SorxResult,
};
use serde_json::Value;

/// Runs SoRX control/observe hooks against signed WASM extension packs loaded by
/// `greentic-ext-runtime`. The binding's `pack_ref` is the extension id.
pub struct WasmExtensionRuntime {
    runtime: Arc<ExtensionRuntime>,
}

impl std::fmt::Debug for WasmExtensionRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmExtensionRuntime").finish_non_exhaustive()
    }
}

impl WasmExtensionRuntime {
    pub fn new(runtime: Arc<ExtensionRuntime>) -> Self {
        Self { runtime }
    }
}

impl RuntimeExtensionAdapter for WasmExtensionRuntime {
    fn control(
        &self,
        hook: &str,
        binding: &RuntimeExtensionBinding,
        request: &Value,
        response: Option<&Value>,
    ) -> SorxResult<ControlDecision> {
        let binding_json = serde_json::to_string(binding)
            .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?;
        let request_json = serde_json::to_string(request)
            .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?;
        let response_json = match response {
            Some(value) => Some(
                serde_json::to_string(value)
                    .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?,
            ),
            None => None,
        };
        let out = self
            .runtime
            .control(
                &binding.pack_ref,
                hook,
                &binding_json,
                &request_json,
                response_json.as_deref(),
            )
            .map_err(|e| SorxError::new("wasm_extension_control_failed", e.to_string()))?;
        serde_json::from_str::<ControlDecision>(&out)
            .map_err(|e| SorxError::new("wasm_extension_decision_invalid", e.to_string()))
    }

    fn observe(
        &self,
        subscription: &str,
        binding: &RuntimeExtensionBinding,
        event: &Value,
    ) -> SorxResult<()> {
        let binding_json = serde_json::to_string(binding)
            .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?;
        let event_json = serde_json::to_string(event)
            .map_err(|e| SorxError::new("wasm_extension_encode_failed", e.to_string()))?;
        self.runtime
            .observe(&binding.pack_ref, subscription, &binding_json, &event_json)
            .map_err(|e| SorxError::new("wasm_extension_observe_failed", e.to_string()))
    }
}
```
Declare the module (feature-gated) in the crate's module root next to the other `mod` lines:
```rust
#[cfg(feature = "wasm-extensions")]
mod wasm_extensions;
```
> `ControlDecision`, `RuntimeExtensionAdapter`, `RuntimeExtensionBinding`, `SorxError`, `SorxResult` must be reachable from `greentic_sorx_core` (they are re-exported — verify; if any is only `pub` in a submodule, import from its path).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p greentic-sorx --features wasm-extensions wasm_extensions 2>&1 | tail -20`
Expected: PASS (2 tests). Then `cargo clippy -p greentic-sorx --all-targets --features wasm-extensions -- -D warnings 2>&1 | tail -5`.

- [ ] **Step 5: Commit**

```bash
git add crates/greentic-sorx-cli/src/wasm_extensions.rs crates/greentic-sorx-cli/src/<module-root>.rs
git commit -m "feat(sorx-cli): WasmExtensionRuntime adapter over greentic-ext-runtime

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Wire into `bind_runtime_extensions` + build the runtime from config

**Files:**
- Modify: `crates/greentic-sorx-cli/src/http_runtime.rs` (`bind_runtime_extensions` gains a wasm-adapter param; a `#[cfg]` builder + call in `from_pack_with_runtime_config`)

**Interfaces:**
- Consumes: `WasmExtensionRuntime` (Task 2); `RuntimeExtensionAdapter` (core trait).
- Produces: `bind_runtime_extensions(runtime, extensions, audit_sink, pack, version, wasm_adapter: Option<Arc<dyn RuntimeExtensionAdapter>>) -> SorxRuntime`.

- [ ] **Step 1: Write the failing wiring test**

Add to `http_runtime.rs`'s `#[cfg(test)] mod tests` (this test runs feature-off too — it uses a native no-op adapter as the "wasm" stand-in to prove registration, so it does not require the git dep):
```rust
    #[test]
    fn bind_runtime_extensions_registers_wasm_adapter_for_nonaudit_packref() {
        use greentic_sorx_core::{
            RuntimeExtensions, RuntimeExtensionBinding, ExtensionFailMode, RuntimeExtensionAdapter,
            ControlDecision,
        };
        use std::sync::Arc;

        // A stand-in adapter that records it was consulted, registered under a
        // non-audit pack_ref, proving bind_runtime_extensions routes wasm packrefs to it.
        #[derive(Debug)]
        struct Spy(std::sync::atomic::AtomicBool);
        impl RuntimeExtensionAdapter for Spy {
            fn control(&self, _h: &str, _b: &RuntimeExtensionBinding, _r: &serde_json::Value, _resp: Option<&serde_json::Value>) -> greentic_sorx_core::SorxResult<ControlDecision> {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(ControlDecision::allow())
            }
        }
        let spy = Arc::new(Spy(std::sync::atomic::AtomicBool::new(false)));

        let mut extensions = RuntimeExtensions::default();
        extensions.control.hooks.insert("pre_call".to_string(), vec![RuntimeExtensionBinding {
            id: "w".into(), contract: "greentic.cap.extension.control.v1".into(),
            pack_ref: "acme.guard.v1".into(), fail_mode: ExtensionFailMode::Closed,
        }]);

        let rt = runtime("test");
        let inner = (*rt.runtime).clone();
        let audit = Arc::new(greentic_sorx_core::MemoryAuditSink::new());
        let bound = bind_runtime_extensions(
            inner, Some(&extensions), audit, "landlord", "1.0.0",
            Some(spy.clone() as Arc<dyn RuntimeExtensionAdapter>),
        );
        // Drive a pre_call through the bound control hook by invoking; the spy must fire.
        // (reuse the same invoke idiom the phase-1 wiring tests use)
        // ... assert spy.0.load(SeqCst) == true after a pre_call invocation
    }
```
> Complete the invoke idiom by copying the phase-1 test `bind_runtime_extensions_wires_bound_observer_end_to_end` (same file) — swap `bound` into `rt.runtime`, drive a business-action `.../invoke` via `handle_request`, then assert `spy.0.load(SeqCst)`. If driving invoke is impractical, assert instead that the returned runtime's control hook is a `BoundControlHook` that resolves `acme.guard.v1` — but prefer the behavioral invoke assertion.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p greentic-sorx bind_runtime_extensions_registers_wasm 2>&1 | tail -20`
Expected: FAIL to compile — `bind_runtime_extensions` takes 5 args, not 6.

- [ ] **Step 3: Implement**

Change `bind_runtime_extensions` (add the param + register the adapter for non-audit pack_refs):
```rust
fn bind_runtime_extensions(
    runtime: SorxRuntime,
    extensions: Option<&greentic_sorx_core::RuntimeExtensions>,
    audit_sink: Arc<dyn greentic_sorx_core::AuditSink>,
    pack: &str,
    version: &str,
    wasm_adapter: Option<Arc<dyn greentic_sorx_core::RuntimeExtensionAdapter>>,
) -> SorxRuntime {
    let Some(extensions) = extensions.filter(|ext| !ext.is_empty()) else {
        return runtime;
    };
    let mut registry = greentic_sorx_core::native_extension_registry(audit_sink, pack, version);
    if let Some(adapter) = wasm_adapter {
        for pack_ref in wasm_pack_refs(extensions) {
            registry = registry.with_adapter(pack_ref, adapter.clone());
        }
    }
    let control = Arc::new(greentic_sorx_core::BoundControlHook::new(
        extensions.clone(),
        registry.clone(),
    ));
    let observer = Arc::new(greentic_sorx_core::BoundObserverHook::new(
        extensions.clone(),
        registry,
    ));
    runtime
        .with_control_hook(control)
        .with_observer_hook(observer, true)
}

/// Distinct pack_refs declared in control hooks + observer subscriptions,
/// excluding the built-in native audit observer.
fn wasm_pack_refs(extensions: &greentic_sorx_core::RuntimeExtensions) -> Vec<String> {
    let mut refs = std::collections::BTreeSet::new();
    for bindings in extensions.control.hooks.values() {
        for b in bindings {
            if b.pack_ref != greentic_sorx_core::NATIVE_AUDIT_PACK_REF {
                refs.insert(b.pack_ref.clone());
            }
        }
    }
    for bindings in extensions.observer.subscriptions.values() {
        for b in bindings {
            if b.pack_ref != greentic_sorx_core::NATIVE_AUDIT_PACK_REF {
                refs.insert(b.pack_ref.clone());
            }
        }
    }
    refs.into_iter().collect()
}
```
Update the ONE existing call to `bind_runtime_extensions` in `from_pack_with_runtime_config` to pass the wasm adapter:
```rust
        let wasm_adapter = build_wasm_extension_adapter(&config);
        let runtime = bind_runtime_extensions(
            runtime,
            runtime_config.as_ref().map(|rc| &rc.extensions),
            audit_sink,
            &pack.pack_name,
            &pack.pack_version,
            wasm_adapter,
        );
```
Add the feature-gated builder (returns `None` when the feature is off or no extensions dir is configured/loadable — so failure to load extensions never breaks startup):
```rust
#[cfg(feature = "wasm-extensions")]
fn build_wasm_extension_adapter(
    _config: &SorxRuntimeConfig,
) -> Option<Arc<dyn greentic_sorx_core::RuntimeExtensionAdapter>> {
    let dir = std::env::var_os("SORX_EXTENSIONS_DIR").map(std::path::PathBuf::from)?;
    let rt_config = greentic_ext_runtime::RuntimeConfig::from_paths(
        greentic_ext_runtime::DiscoveryPaths::new(dir),
    );
    match greentic_ext_runtime::ExtensionRuntime::new(rt_config) {
        Ok(runtime) => Some(Arc::new(crate::wasm_extensions::WasmExtensionRuntime::new(
            Arc::new(runtime),
        ))),
        Err(err) => {
            // Do not fail startup if extensions can't be loaded; log and run native-only.
            eprintln!("wasm extension runtime disabled: {err}");
            None
        }
    }
}

#[cfg(not(feature = "wasm-extensions"))]
fn build_wasm_extension_adapter(
    _config: &SorxRuntimeConfig,
) -> Option<Arc<dyn greentic_sorx_core::RuntimeExtensionAdapter>> {
    None
}
```
> `eprintln!` is used because `greentic-sorx-core` has no tracing dep (consistent with phase-1). If sorx-cli already uses `tracing`, prefer `tracing::warn!`. Confirm `SorxRuntimeConfig` is the right type name in scope at that call site.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p greentic-sorx bind_runtime_extensions 2>&1 | tail -20` (feature off — wiring test uses the native Spy, passes without the git dep).
Run: `cargo test -p greentic-sorx --features wasm-extensions 2>&1 | tail -10` and `cargo test -p greentic-sorx 2>&1 | tail -10` — both green, no regressions.

- [ ] **Step 5: Commit**

```bash
git add crates/greentic-sorx-cli/src/http_runtime.rs
git commit -m "feat(sorx-cli): register WASM extension adapter in bind_runtime_extensions

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Docs

**Files:**
- Modify: `docs/extensions.md` (phase-1 doc)

- [ ] **Step 1: Document the WASM path**

Append a section to `docs/extensions.md`:
```markdown
## WASM extension packs (phase 2)

Behind the `wasm-extensions` cargo feature (off by default), SoRX runs signed WASM
extension packs via `greentic-ext-runtime` (world `greentic:extension-sorx`). A binding's
`pack_ref` is the extension id; the operator installs the signed extension directory and
points `SORX_EXTENSIONS_DIR` at the discovery root (default `~/.greentic/extensions/sorx/`).
Unsigned local dev uses `greentic-ext-runtime`'s `dev-allow-unsigned` feature +
`GREENTIC_EXT_ALLOW_UNSIGNED=1`. With the feature off, only native adapters (the audit
observer) are available and no wasmtime dependency is compiled in.
```

- [ ] **Step 2: Commit**

```bash
git add docs/extensions.md
git commit -m "docs(ext): document WASM extension packs (phase 2)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Gate (both feature states) + PR

- [ ] **Step 1: Full gates**

```bash
cargo fmt --all -- --check
cargo clippy -p greentic-sorx --all-targets -- -D warnings
cargo clippy -p greentic-sorx --all-targets --features wasm-extensions -- -D warnings
cargo test -p greentic-sorx-core --lib
cargo test -p greentic-sorx
cargo test -p greentic-sorx --features wasm-extensions
```
Expected: all green (feature on AND off). Slow — background + WAIT. Do NOT run `--features foundationdb`; ignore the CI `perf` job.

- [ ] **Step 2: Push + PR into research**

```bash
git push -u origin feat/sorx-wasm-extension-adapter
gh pr create --base research --head feat/sorx-wasm-extension-adapter \
  --title "feat(sorx): WASM extension adapter over greentic-ext-runtime (phase-2 Sub-B)" \
  --body "..."   # summarize: feature-gated git dep on greentic-ext-runtime@c28e957; WasmExtensionRuntime adapter (pack_ref=ext id) wired into bind_runtime_extensions; core stays wasmtime-free; adapter-level tests; real WASM execution deferred to Sub-C; note the sync-blocking consideration for prod.
```
