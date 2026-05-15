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

Locked business action `invoke` calls the same endpoint runtime path after
id/version/hash and payload validation. The response includes the action ref and
runtime events returned by the underlying endpoint invocation. Locked business
action `dry-run` does not emit provider-operation events because it does not
mutate provider state.

Locked business action invoke responses include audit metadata for the selected
action id, version, expected contract hash, validation result, result status,
and underlying runtime events. Payload values are not copied into audit
metadata.

Additional event names used by current flows:

- `sorx.approval.requested`
- `sorx.endpoint.failed`

Ontology-aware command outputs include an `audit_events` array using the stable
`greentic.sorx.ontology.audit.v1` schema. Current ontology event names:

- `ontology.graph.loaded`
- `ontology.path.resolved`
- `provider.compatibility.checked`
- `evidence.query.planned`
- `evidence.query.executed`
- `entity.links.resolved`
- `policy.ontology.decision`
- `action.ontology.executed`
- `public.exposure.gated`

Ontology explain payloads expose graph hashes, concepts, relationships,
providers, evidence IDs, policy decisions, and redaction metadata.
- `ontology.graph.loaded`
- `ontology.path.resolved`
- `evidence.query.planned`
- `evidence.query.executed`

Ontology graph and evidence commands include deterministic `audit_events`
arrays in their JSON output. These command-level events use ontology hashes,
IDs, and counts rather than request bodies.

Planned deployment lifecycle events:

- `sorx.pack.loaded`
- `sorx.route.registered`
- `sorx.tool.registered`
- `sorx.request.received`

Audit events intentionally record metadata, not request bodies. Secret-like
fields in ontology audit details are redacted before serialization.
