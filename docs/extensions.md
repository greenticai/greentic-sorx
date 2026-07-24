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
invocation.

Phase 2 (future) adds a WASM component adapter for third-party extension packs.
