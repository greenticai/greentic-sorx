# PR-04 — Add metrics audit, caching and policy controls in SORX

Repository: `greenticai/greentic-sorx`

## Goal

Make metric queries production-safe by adding audit events, optional caching metadata and policy checks.

## Current repo validation

The repo already has endpoint audit and policy infrastructure:

- `SorxRuntime` emits structured endpoint/provider/policy/approval audit events through `AuditSink`.
- `PolicyEngine` currently reasons about `EndpointDefinition` risk/action, not generic metric action strings.
- HTTP runtime has SORX-owned routes and can call runtime services directly.

Design update:

- Do not create a second unrelated audit system for metrics. Reuse `AuditSink` and event naming patterns from `runtime.rs`.
- Either map metric operations to pseudo endpoint definitions for the existing `PolicyEngine`, or extend policy with metric-specific decisions in a way that preserves existing endpoint behavior.
- Keep request body/data minimization consistent with existing audit behavior; metric filters may contain sensitive values.
- Parse and surface cache hints with metric metadata first. Full caching can remain deferred unless there is a concrete storage and invalidation design.
- If caching is implemented, include tenant/environment and the full normalized query in the key.

## Audit

Emit audit events for:

- metric listed
- metric queried
- query rejected by policy
- provider capability missing
- formula evaluation failed
- cache hit/miss if caching is enabled

## Policy

Add policy hooks:

```text
metrics.list
metrics.read_definition
metrics.query
metrics.query_sensitive_dimension
metrics.query_large_range
```

Large range threshold can be configurable.

## Caching

MVP can expose cache hints without implementing a full cache.

Metric definition may include:

```json
{
  "cache": {
    "ttl_seconds": 300,
    "scope": "tenant"
  }
}
```

If cache is implemented, cache key must include metric, time range, grain, dimensions, filters and tenant.

## Acceptance criteria

- Metric queries emit structured audit events.
- Policy checks can deny metric queries.
- Sensitive dimensions can trigger stricter policy.
- Cache hints are parsed and surfaced, even if full cache is deferred.
- Tests cover audit and deny paths.
