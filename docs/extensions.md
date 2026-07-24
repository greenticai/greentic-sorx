# Runtime extensions (phase 1: native)

SoRX dispatches declared extension bindings through the control/observer seams.
Bindings are declared in `RuntimeConfig.extensions` (runtime config JSON), not in
the pack. When present they are wired live; when absent the runtime uses no-op
hooks (identical to a deployment without extensions).

## Built-in: audit observer

`pack_ref: greentic.sorx.audit.v1` records `pre_call` / `post_call` /
`call_failed` / `control_denied` observer events to the runtime `AuditSink` as
`SorxAuditEvent`s (`event = "observer.<subscription>"`). It is a coarse companion
to the invoke-time audit stream: `operation_id` mirrors the route and `risk` is
`Low`, with the full (redacted) call context under `details`.

Enable it by declaring observer subscriptions, e.g.:

```json
{ "extensions": { "observer": { "subscriptions": {
  "pre_call":  [{ "id": "audit-pre",  "contract": "greentic.cap.extension.observer.v1", "pack_ref": "greentic.sorx.audit.v1", "fail_mode": "open" }],
  "post_call": [{ "id": "audit-post", "contract": "greentic.cap.extension.observer.v1", "pack_ref": "greentic.sorx.audit.v1", "fail_mode": "open" }]
} } } }
```

Use `fail_mode: "open"` for audit so an audit failure never fails the business
invocation. Note that observer bindings are effectively fail-open at the
runtime layer regardless of their declared `fail_mode`: the runtime discards
observer errors, so audit can never fail a business call; the per-binding
`fail_mode` is honored for the control path.

## WASM extension packs (phase 2)

Behind the `wasm-extensions` cargo feature (off by default), SoRX runs signed WASM
extension packs via `greentic-ext-runtime` (world `greentic:extension-sorx`). A binding's
`pack_ref` is the extension id; the operator installs the signed extension directory and
points `SORX_EXTENSIONS_DIR` at the discovery root (default `~/.greentic/extensions/sorx/`).
Unsigned local dev uses `greentic-ext-runtime`'s `dev-allow-unsigned` feature +
`GREENTIC_EXT_ALLOW_UNSIGNED=1`. With the feature off, only native adapters (the audit
observer) are available and no wasmtime dependency is compiled in.
