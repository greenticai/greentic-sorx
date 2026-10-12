# SoRX WASM Extension E2E — Implementation Plan (Sub-C)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove SoRX actually executes WASM extension packs: a `wasm32-wasip2` guest implementing `greentic:extension-sorx` (control/observe), loaded dev-unsigned and invoked through `WasmExtensionRuntime` (Sub-B) in a feature-gated end-to-end test that asserts the guest's real `ControlDecision`.

**Architecture:** guest built with `cargo-component` (template = `telco-x-designer-ext`), lives as a non-workspace fixture; e2e test (feature `wasm-extensions-dev-unsigned`) builds it, assembles a `describe.json` + `extension.wasm` dir, loads it via `greentic-ext-runtime` with `GREENTIC_EXT_ALLOW_UNSIGNED=1`, dispatches control/observe. Default CI is unaffected (test only compiles under the opt-in feature). `spawn_blocking` hardening is a no-op (HTTP path is `std::thread`-isolated; NATS bridge already `spawn_blocking`s invoke) — documented only.

**Tech Stack:** Rust 1.95, `cargo-component 0.21` + `wasm32-wasip2` + `wit-bindgen 0.41`, `greentic-ext-runtime` (git, Sub-A), `serde_json`, `tempfile`.

## Global Constraints

- Rust 1.95.0; `#![forbid(unsafe_code)]` norm; no `unwrap()`/`panic!()` in production (test code may unwrap); `SorxResult`/`SorxError`.
- English only; Conventional Commits; sorx allows the `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer.
- **Default build/CI unchanged:** the e2e test + its deps compile ONLY under `wasm-extensions-dev-unsigned`. `cargo test -p greentic-sorx` (default) and the merge gate (`ci.yml`, no `--all-features`) must be byte-unaffected.
- `dev-allow-unsigned` is TEST-ONLY — never in a production/default feature.
- The guest fixture is a NON-workspace member (different target, built by cargo-component) — exclude it so host `cargo build` never touches it.
- WIT/cargo-component wiring is iterative: mirror `telco-x-designer-ext` (`/home/bima-pangestu/projects/Works/greentic/telco-x-designer-ext/{Cargo.toml,src/lib.rs,wit/}`) and fix any `cargo component build` error against that example — the generated `src/bindings.rs` is gitignored.

**Reference facts (verified):**
- `WasmExtensionRuntime` (Sub-B, `greentic-sorx-cli/src/wasm_extensions.rs`): `pub fn new(runtime: Arc<greentic_ext_runtime::ExtensionRuntime>) -> Self`; `impl RuntimeExtensionAdapter` with `control(hook, binding, request, response) -> SorxResult<ControlDecision>` / `observe(...)`.
- `ExtensionRuntime::new(RuntimeConfig::from_paths(DiscoveryPaths::new(user: PathBuf))) -> Result<Self, RuntimeError>`; loading verifies signature UNLESS `#[cfg(feature="dev-allow-unsigned")]` + `GREENTIC_EXT_ALLOW_UNSIGNED` env → skips verify (and manifest). `describe.json` JSON-Schema validation always runs.
- ext dir layout: `<root>/<anything>/describe.json` + `<root>/<...>/extension.wasm`. `DiscoveryPaths::new(<root>)` discovers dirs containing `describe.json`.
- The extension id in `describe.json.metadata.id` == the binding `pack_ref` == the ext_id `WasmExtensionRuntime` passes to `ExtensionRuntime::control`.

---

### Task 1: Guest wasm32-wasip2 component

**Files (all new, under `tests/fixtures/sorx-e2e-guest/`, a NON-workspace crate):**
- `Cargo.toml`, `src/lib.rs`, `wit/world.wit`, `wit/deps/greentic/extension-sorx/extension-sorx.wit`, `wit/deps/greentic/extension-host/extension-host.wit`, `.gitignore`
- Modify: root `Cargo.toml` — add `tests/fixtures/sorx-e2e-guest` to `[workspace] exclude`.

**Interfaces:**
- Produces: a component that, built with `cargo component build --release --target wasm32-wasip2`, exports `greentic:extension-sorx/control@0.1.0` (returns `{"action":"deny",...}` when `request-json` contains `"deny":true`, else `{"action":"allow"}`) and `greentic:extension-sorx/observe@0.1.0` (logs + `Ok(())`).

- [ ] **Step 1: Scaffold + vendor WIT**

Root `Cargo.toml`: add the exclude:
```toml
[workspace]
members = [ "crates/greentic-sorx-cli", "crates/greentic-sorx-core", "crates/greentic-sorx-pack", "crates/sorx-event-bridge" ]
exclude = [ "tests/fixtures/sorx-e2e-guest" ]
resolver = "3"
```

`tests/fixtures/sorx-e2e-guest/Cargo.toml`:
```toml
[package]
name = "sorx-e2e-guest"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen-rt = { version = "0.41", features = ["bitflags"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[package.metadata.component]
package = "greentic:sorx-e2e-guest"

[package.metadata.component.target]
path = "wit"

[package.metadata.component.target.dependencies]
"greentic:extension-sorx" = { path = "wit/deps/greentic/extension-sorx" }
"greentic:extension-host" = { path = "wit/deps/greentic/extension-host" }
```

`tests/fixtures/sorx-e2e-guest/wit/world.wit`:
```wit
package greentic:sorx-e2e-guest@0.1.0;

world guest {
  import greentic:extension-host/logging@0.1.0;
  export greentic:extension-sorx/control@0.1.0;
  export greentic:extension-sorx/observe@0.1.0;
}
```

`wit/deps/greentic/extension-sorx/extension-sorx.wit` — copy verbatim from the pinned checkout
`~/.cargo/git/checkouts/greentic-designer-extensions-*/c28e957/crates/greentic-ext-runtime/wit/deps/extension-sorx/extension-sorx.wit`
(package `greentic:extension-sorx@0.1.0` with `interface control`, `interface observe`, and its own `world sorx-runtime-extension`; the guest's own `world guest` above is what cargo-component builds).

`wit/deps/greentic/extension-host/extension-host.wit` — a MINIMAL vendored package with just the imported interface (avoids pulling `extension-base`):
```wit
package greentic:extension-host@0.1.0;

interface logging {
  enum level { trace, debug, info, warn, error }
  log: func(level: level, target: string, message: string);
  log-kv: func(level: level, target: string, message: string, fields: list<tuple<string, string>>);
}
```

`.gitignore`:
```
/src/bindings.rs
/target
```

- [ ] **Step 2: Guest logic**

`tests/fixtures/sorx-e2e-guest/src/lib.rs`:
```rust
#[allow(warnings)]
mod bindings;

use bindings::exports::greentic::extension_sorx::{control, observe};
use bindings::greentic::extension_host::logging;

struct Component;

impl control::Guest for Component {
    fn control(
        _hook: String,
        _binding_json: String,
        request_json: String,
        _response_json: Option<String>,
    ) -> Result<String, String> {
        let deny = serde_json::from_str::<serde_json::Value>(&request_json)
            .ok()
            .and_then(|v| v.get("deny").and_then(|d| d.as_bool()))
            .unwrap_or(false);
        if deny {
            Ok(r#"{"action":"deny","reason":"e2e guest denied"}"#.to_string())
        } else {
            Ok(r#"{"action":"allow"}"#.to_string())
        }
    }
}

impl observe::Guest for Component {
    fn observe(subscription: String, _binding_json: String, _event_json: String) -> Result<(), String> {
        logging::log(logging::Level::Info, "sorx-e2e-guest", &format!("observed {subscription}"));
        Ok(())
    }
}

bindings::export!(Component with_types_in bindings);
```
> The exact bindings module paths (`exports::greentic::extension_sorx::{control,observe}`, `greentic::extension_host::logging`, `logging::Level`) are what wit-bindgen generates for these package/interface names; if a path differs, run `cargo component build` once and read the generated `src/bindings.rs` (or the compiler error) to correct the `use` paths — mirror how `telco-x-designer-ext/src/lib.rs` imports from its `bindings`.

- [ ] **Step 3: Build the guest**

Run (from the fixture dir): `cd tests/fixtures/sorx-e2e-guest && cargo component build --release --target wasm32-wasip2 2>&1 | tail -30`
Expected: a `target/wasm32-wasip2/release/sorx_e2e_guest.wasm` is produced. Iterate on WIT/paths against `cargo component build` errors + the telco-x example until it builds. (This is the inherently iterative step.)
Verify it is a component: `wasm-tools component wit target/wasm32-wasip2/release/sorx_e2e_guest.wasm 2>&1 | grep -E 'extension-sorx|control|observe'` — should list the exported control/observe.

- [ ] **Step 4: Commit**

```bash
git add tests/fixtures/sorx-e2e-guest Cargo.toml
git commit -m "test(sorx): wasm32-wasip2 guest fixture for the extension-sorx world

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: End-to-end execution test

**Files:**
- Modify: `crates/greentic-sorx-cli/Cargo.toml` — add feature `wasm-extensions-dev-unsigned` + `tempfile` dev-dep (if not present).
- Create/Modify: `crates/greentic-sorx-cli/tests/wasm_extension_e2e.rs` (a `#![cfg(feature = "wasm-extensions-dev-unsigned")]` integration test).

**Interfaces:**
- Consumes: the guest (Task 1); `WasmExtensionRuntime`, `greentic_ext_runtime::{ExtensionRuntime, RuntimeConfig, DiscoveryPaths}`; `greentic_sorx_core::{RuntimeExtensionBinding, ExtensionFailMode, ControlDecisionAction, RuntimeExtensionAdapter}`.

- [ ] **Step 1: Feature + dev-dep**

In `crates/greentic-sorx-cli/Cargo.toml`:
```toml
[features]
wasm-extensions-dev-unsigned = ["wasm-extensions", "greentic-ext-runtime/dev-allow-unsigned"]
```
Ensure `[dev-dependencies]` has `tempfile` (check; add `tempfile = "3"` if missing).

- [ ] **Step 2: Write the failing test**

`crates/greentic-sorx-cli/tests/wasm_extension_e2e.rs`:
```rust
#![cfg(feature = "wasm-extensions-dev-unsigned")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use greentic_ext_runtime::{DiscoveryPaths, ExtensionRuntime, RuntimeConfig};
use greentic_sorx_core::{
    ControlDecisionAction, ExtensionFailMode, RuntimeExtensionAdapter, RuntimeExtensionBinding,
};
use greentic_sorx::wasm_extensions::WasmExtensionRuntime; // adjust to the actual pub path

const EXT_ID: &str = "greentic.sorx.e2e-guest";

fn guest_wasm() -> PathBuf {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/sorx-e2e-guest");
    let status = Command::new("cargo")
        .args(["component", "build", "--release", "--target", "wasm32-wasip2"])
        .current_dir(&fixture)
        .status()
        .expect("cargo-component must be installed to run this opt-in e2e test");
    assert!(status.success(), "guest build failed");
    fixture.join("target/wasm32-wasip2/release/sorx_e2e_guest.wasm")
}

fn describe_json() -> String {
    // minimal schema-valid describe.json; sha256 placeholders (unsigned path never checks bytes)
    format!(r#"{{
      "apiVersion":"greentic.ai/v2","kind":"ProviderExtension",
      "compat":{{"min_designer_version":">=1.0.0","min_runner_version":"^0.12.0","contract_version":"1.2.0"}},
      "metadata":{{"id":"{EXT_ID}","name":"{EXT_ID}","version":"0.1.0","summary":"sorx e2e guest","author":{{"name":"test"}},"license":"MIT"}},
      "engine":{{"greenticDesigner":"*","extRuntime":"*"}},
      "capabilities":{{"offered":[],"required":[]}},
      "runtime":{{"permissions":{{"network":[],"secrets":[],"callExtensionKinds":[]}},
        "components":{{"sorx-guest":{{"gtpack":{{"file":"extension.wasm","sha256":"{Z}","pack_id":"{EXT_ID}","component_version":"0.1.0"}},"sha256":"{Z}","world":"greentic:extension-sorx/sorx-runtime-extension@0.1.0"}}}}}},
      "contributions":{{}}
    }}"#, Z = "0".repeat(64))
}

fn binding() -> RuntimeExtensionBinding {
    RuntimeExtensionBinding {
        id: "e2e".into(),
        contract: "greentic.cap.extension.control.v1".into(),
        pack_ref: EXT_ID.into(),
        fail_mode: ExtensionFailMode::Closed,
    }
}

#[test]
fn guest_control_and_observe_execute_end_to_end() {
    let wasm = guest_wasm();
    let root = tempfile::TempDir::new().unwrap();
    let dir = root.path().join(EXT_ID);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&wasm, dir.join("extension.wasm")).unwrap();
    std::fs::write(dir.join("describe.json"), describe_json()).unwrap();

    // SAFETY: single-threaded test process; the dev-unsigned gate reads this env var.
    unsafe { std::env::set_var("GREENTIC_EXT_ALLOW_UNSIGNED", "1") };

    let rt = ExtensionRuntime::new(RuntimeConfig::from_paths(DiscoveryPaths::new(
        root.path().to_path_buf(),
    )))
    .expect("load unsigned guest");
    let adapter = WasmExtensionRuntime::new(Arc::new(rt));

    // deny path — the guest returns a real deny decision:
    let denied = adapter
        .control("pre_call", &binding(), &serde_json::json!({"deny": true}), None)
        .expect("control dispatch");
    assert_eq!(denied.action, ControlDecisionAction::Deny);
    assert!(denied.reason.as_deref().unwrap_or("").contains("denied"));

    // allow path:
    let allowed = adapter
        .control("pre_call", &binding(), &serde_json::json!({}), None)
        .expect("control dispatch");
    assert_eq!(allowed.action, ControlDecisionAction::Allow);

    // observe runs (logs + Ok):
    adapter
        .observe("post_call", &binding(), &serde_json::json!({"event_type": "stack.call.completed"}))
        .expect("observe dispatch");
}
```
> Adjust `use greentic_sorx::wasm_extensions::WasmExtensionRuntime;` to the actual crate/module path (the cli crate is package `greentic-sorx`, lib name `greentic_sorx_cli` — confirm and import accordingly; `WasmExtensionRuntime`/`wasm_extensions` may need to be `pub` and re-exported from the lib root, which the Sub-B `pub mod wasm_extensions;` already allows). If `ExtensionRuntime::new` needs the dir registered explicitly rather than discovered, use `register_loaded_from_dir(&dir)` instead of relying on `DiscoveryPaths` discovery (discovery scans kind-subdirs; if the id-named dir isn't discovered, register it directly). Resolve against the pinned ext-runtime `discovery.rs`.

- [ ] **Step 3: Run the test**

Run: `GREENTIC_EXT_ALLOW_UNSIGNED=1 cargo test -p greentic-sorx --features wasm-extensions-dev-unsigned --test wasm_extension_e2e 2>&1 | tail -30`
Expected: the guest builds, loads, and the three assertions pass. Iterate on describe.json/discovery/import-paths until green. (First run also cold-builds the wasmtime tree — slow; background + WAIT.)

- [ ] **Step 4: Confirm default build is unaffected**

Run: `cargo test -p greentic-sorx 2>&1 | tail -5` (default, no feature) — the e2e test is `#![cfg(feature)]`-gated so it does not compile here; existing 197 tests still pass.

- [ ] **Step 5: Commit**

```bash
git add crates/greentic-sorx-cli/Cargo.toml crates/greentic-sorx-cli/tests/wasm_extension_e2e.rs
git commit -m "test(sorx): end-to-end WASM control/observe execution (dev-unsigned)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Docs + build helper + async-safety invariant

**Files:**
- Modify: `docs/extensions.md`
- Create: `ci/build-sorx-e2e-guest.sh`

- [ ] **Step 1: Docs**

Append to `docs/extensions.md`:
```markdown
### End-to-end test (opt-in)

`cargo test -p greentic-sorx --features wasm-extensions-dev-unsigned --test wasm_extension_e2e`
builds the `tests/fixtures/sorx-e2e-guest` component (needs `cargo-component` + the `wasm32-wasip2`
target), loads it dev-unsigned (`GREENTIC_EXT_ALLOW_UNSIGNED=1`), and dispatches real control/observe.
Default CI does not run it.

### Async-safety invariant

`WasmExtensionRuntime::{control,observe}` and `SorxRuntime::invoke` are **synchronous and may block**
(wasmtime store calls). The HTTP server runs each request on a dedicated `std::thread`, so no async
reactor is stalled. Any *async* caller of `SorxRuntime::invoke` (or of the adapter directly) MUST
dispatch on `tokio::task::spawn_blocking`, as the NATS event bridge already does
(`crates/greentic-sorx-cli/src/event_bridge_invoker.rs`).
```

- [ ] **Step 2: Build helper**

`ci/build-sorx-e2e-guest.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../tests/fixtures/sorx-e2e-guest"
cargo component build --release --target wasm32-wasip2
echo "built: $(pwd)/target/wasm32-wasip2/release/sorx_e2e_guest.wasm"
```
`chmod +x ci/build-sorx-e2e-guest.sh`.

- [ ] **Step 3: Commit**

```bash
git add docs/extensions.md ci/build-sorx-e2e-guest.sh
git commit -m "docs(ext): e2e test how-to + WASM async-safety invariant

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Gate + PR

- [ ] **Step 1: Gates**

```bash
cargo fmt --all -- --check
cargo clippy -p greentic-sorx --all-targets -- -D warnings
cargo test -p greentic-sorx            # default: unchanged, e2e not compiled
GREENTIC_EXT_ALLOW_UNSIGNED=1 cargo test -p greentic-sorx --features wasm-extensions-dev-unsigned --test wasm_extension_e2e
```
Expected: default gate green + unchanged; the e2e test green (needs cargo-component). Ignore the CI `perf` job; do NOT run `--features foundationdb`.

- [ ] **Step 2: Push + PR into research**

```bash
git push -u origin feat/sorx-wasm-extension-e2e
gh pr create --base research --head feat/sorx-wasm-extension-e2e \
  --title "test(sorx): end-to-end WASM extension execution + guest fixture (phase-2 Sub-C)" \
  --body "..."   # summarize: wasm32-wasip2 guest fixture + dev-unsigned e2e proving real control(deny/allow)/observe execution and interface-id resolution; opt-in feature (default CI unaffected); spawn_blocking hardening confirmed no-op (HTTP=std::thread, NATS already spawn_blocking) + documented. Note the e2e job needs cargo-component + a read token for the greentic-ext-runtime git dep.
```
