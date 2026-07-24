# SoRX native extension dispatch + audit observer — design

_Date: 2026-07-24 · Repo: `greentic-sorx` · Branch: `feat/sorx-native-extension-dispatch` (off `research`)_

## Context

SoRLa/SoRX productionization epic, item #6 (agentic interface), **part B**, **phase 1**.

The SoRX runtime-extension framework is coherent but **inert in production**:

- `RuntimeExtensionAdapter` (`greentic-sorx-core/src/generic_runtime.rs`) is a JSON-in/JSON-out
  trait keyed by `pack_ref`, with `control()` and `observe()` methods.
- `RuntimeExtensionRegistry` maps `pack_ref → Arc<dyn RuntimeExtensionAdapter>`.
- `BoundControlHook` / `BoundObserverHook` bridge the typed `ControlHook`/`ObserverHook`
  seams to the registry, honoring per-binding `fail_mode` (Open = swallow+continue,
  Closed = propagate).
- **But** `SorxRuntime` is constructed with `NoopControlHook` / `NoopObserverHook`, and the
  only code that ever wires a `Bound*Hook` lives under `#[cfg(test)]`. Extension bindings
  declared in `RuntimeConfig.extensions` are validated and snapshotted but **never dispatched**.

There is **no execution substrate inside sorx** (no wasmtime / WIT). A production WASM
component host exists elsewhere (`greentic-ext-runtime`, `greentic-component-runtime`) and is
the phase-2 reuse target; it is explicitly **out of scope here**.

## Goal (phase 1)

Make the extension seam **live in production** and prove it end-to-end with one useful,
low-risk built-in adapter, activated opt-in via configuration.

Success criteria:

1. When a deployment's `RuntimeConfig.extensions` declares an observer subscription to the
   built-in audit adapter, invoking an action (over HTTP or MCP) produces structured audit
   records through the existing `AuditSink`.
2. When no extension bindings are declared, runtime behavior is byte-identical to today
   (no dispatch, no overhead) — verified by a parity test.

## Non-goals (YAGNI)

- No WASM / WIT / component host (phase 2).
- No pack-loader binding parsing — bindings come from `RuntimeConfig.extensions` (config JSON),
  matching the existing model. The pack loader keeps validating only the
  `greentic.sorx.runtime.v1` marker + asset references.
- No native **control** adapter shipped (the control path is wired live, but phase 1 ships
  only an observer adapter).
- No new capability schemas.

## Architecture

### Components (in `greentic-sorx-core`)

**`NativeAuditObserver`** — `impl RuntimeExtensionAdapter`.
- Holds `Arc<dyn AuditSink>` (the existing sink trait, already used by `SorxRuntime`).
- `observe(subscription, binding, event)`: maps the subscription name
  (`pre_call` / `post_call` / `call_failed` / `control_denied`) and the event JSON into a
  `SorxAuditEvent`, redacting secret-like fields via the existing `redact_audit_value`, then
  calls `sink.record(event)`. Returns `Ok(())` unless the sink hard-fails.
- `control()`: default (allow) — this is an observer, not a control.
- Advertised `pack_ref`: `greentic.sorx.audit.v1`.

**`native_extension_registry(audit_sink: Arc<dyn AuditSink>) -> RuntimeExtensionRegistry`** —
constructs a registry pre-populated with the built-in adapters. Phase 1 registers exactly one:
`greentic.sorx.audit.v1 → NativeAuditObserver`. This is the single place future built-ins are
added.

### Wiring (in `greentic-sorx-cli`, `HttpRuntime::from_pack_with_runtime_config`)

Construction today (`http_runtime.rs`): `SorxRuntime::new(...)` is wrapped by
`configure_runtime_audit(runtime, &config)` (which attaches the `AuditSink`); the
`runtime_config: Option<RuntimeConfig>` parameter carries `.extensions` (the bindings).

Change: after the audit sink is attached, if `runtime_config` declares any extension bindings,
wrap the runtime's `RuntimeExtensions` in `BoundControlHook` / `BoundObserverHook` (built against
`native_extension_registry(<the audit sink>)`) and inject via `SorxRuntime::with_control_hook` /
`with_observer_hook`.

- **Parity / non-breaking:** when no bindings are declared, keep the `Noop*Hook`s — behavior is
  byte-identical to today. (Wrapping empty bindings would also be a no-op per the dispatch
  logic, but keeping Noop makes the parity explicit and avoids any allocation.)
- The audit sink handle must be shared between the runtime and the observer. Implementation
  detail for the plan: build the `Arc<dyn AuditSink>` once, pass a clone to both
  `configure_runtime_audit` (or its underlying sink construction) and `native_extension_registry`.
- The **control path is wired live** for completeness. Phase 1 ships no native control adapter,
  so a control binding referencing an unknown `pack_ref` fails per its `fail_mode`
  (fail-closed → deny) — the correct, safe default. Default configs declare no control bindings.

### Data flow

```
agent (MCP #6-A) / HTTP  →  SorxRuntime.invoke
  →  observer_hook.observe(ObserverEvent)           (runtime.rs)
  →  BoundObserverHook.notify(subscription, event)
       for each binding with pack_ref = greentic.sorx.audit.v1:
  →  registry.adapter(pack_ref)  →  NativeAuditObserver.observe
  →  SorxAuditEvent (redacted)   →  AuditSink.record
```

### Error handling

Dispatch honors `binding.fail_mode`:
- `Open`: a failing adapter is logged and skipped; the invocation proceeds.
- `Closed` (default): a failing adapter propagates → the invocation is denied/failed.

Recommended (documented) config for the audit binding: `fail_mode: Open` — an audit failure
must not fail the business invocation. `NativeAuditObserver` itself only errors if the sink
hard-fails.

## Testing (TDD)

Unit (`greentic-sorx-core`):
- `NativeAuditObserver.observe` produces the correct `SorxAuditEvent` for each subscription
  (`pre_call`, `post_call`, `call_failed`, `control_denied`).
- Secret-like fields in the event payload are redacted in the recorded audit event.

Integration (`greentic-sorx-cli`, the phase-1 proof):
- Build an `HttpRuntime` with a `MemoryAuditSink` and a `RuntimeConfig` declaring an observer
  subscription (`pre_call` + `post_call`) to `greentic.sorx.audit.v1`; invoke an action via
  `handle_request`; assert the `MemoryAuditSink` captured the expected pre_call/post_call
  records.
- **Parity:** the same runtime with no extension bindings records nothing and dispatches
  through `Noop*Hook`.

## Files touched

- `greentic-sorx-core/src/` — new `NativeAuditObserver` + `native_extension_registry`
  (likely a new small module, e.g. `native_extensions.rs`, re-exported from `lib.rs`); unit tests.
- `greentic-sorx-cli/src/http_runtime.rs` — wire `Bound*Hook` in
  `from_pack_with_runtime_config`; integration + parity tests.
- Docs: a short note in `docs/` (extension dispatch behavior + recommended `fail_mode: Open`
  for audit bindings) and `.codex/repo_overview.md` if present.

## Phase 2 (not this spec)

WASM component-model adapter: define a `sorx-runtime-extension` WIT world (control/observe),
reuse the `greentic-ext-runtime` engine/linker/signature-verify machinery, implement
`RuntimeExtensionAdapter for WasmExtensionRuntime` resolving `pack_ref → loaded component`, and
plug it into the same registry built here.
