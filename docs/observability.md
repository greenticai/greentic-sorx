# Observability

SORX emits structured audit events through an `AuditSink`. The local HTTP
runtime currently supports `stdout` and disabled audit sinks, while tests use an
in-memory sink.

Event names used by the runtime today:

```json
{
  "event": "sorx.endpoint.invoked",
  "pack": "landlord-tenant-sor",
  "version": "0.1.0",
  "tenant_id": "tenant-a",
  "endpoint_id": "tenant.create",
  "operation_id": "tenant.create",
  "risk": "medium",
  "caller_id": "tester",
  "decision": null,
  "duration_ms": null,
  "idempotency_key_present": true,
  "details": {
    "source": "http"
  }
}
```

Runtime event sequence for an executed provider operation:

1. `sorx.endpoint.invoked`
2. `sorx.policy.decided`
3. `sorx.provider.operation.started`
4. `sorx.provider.operation.completed`
5. `sorx.endpoint.completed`

Additional event names used by current flows:

- `sorx.approval.requested`
- `sorx.endpoint.failed`

Planned deployment lifecycle events:

- `sorx.pack.loaded`
- `sorx.route.registered`
- `sorx.tool.registered`
- `sorx.request.received`

Audit events intentionally record metadata, not request bodies.
