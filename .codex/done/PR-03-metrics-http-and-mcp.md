# PR-03 — Expose metrics through SORX HTTP routes and MCP tools

Repository: `greenticai/greentic-sorx`

## Goal

Expose declared metrics through SORX runtime APIs.

## Current repo validation

HTTP routing currently lives in `crates/greentic-sorx-cli/src/http_runtime.rs`.

Existing SORX-owned diagnostic/runtime routes use `/v1/sorx/...`, while pack agent endpoints come from `assets/sorla/agent-gateway.json`. MCP tool definitions are currently loaded from `assets/sorla/mcp-tools.json` and normalized through `greentic_sorx_core::mcp_tools_from_metadata`.

Design update:

- Prefer SORX-owned metric routes under `/v1/sorx/metrics` unless there is a deliberate reason to expose pack-owned `/metrics` routes. Bare `/metrics` may collide with conventional Prometheus/server telemetry endpoints in future.
- Keep metric HTTP routes separate from agent-gateway endpoint matching unless metric definitions are intentionally modeled as gateway endpoints.
- Generated MCP metric tools should be deterministic and clearly namespaced, for example `sorx.metrics.list`, `sorx.metrics.get`, and `sorx.metrics.query`, or stable per-metric names if the existing MCP style strongly favors per-endpoint tools.
- Existing policy is endpoint/risk based. Sensitive metric query handling should either create explicit pseudo-endpoint metadata for policy decisions or wait for PR-04's metric policy hooks. Do not silently bypass policy.

## HTTP routes

Add routes such as:

```text
GET /v1/sorx/metrics
GET /v1/sorx/metrics/{metric_name}
POST /v1/sorx/metrics/{metric_name}/query
```

Example query body:

```json
{
  "from": "2026-01-01T00:00:00Z",
  "to": "2026-02-01T00:00:00Z",
  "grain": "day",
  "dimensions": ["campaign_id"],
  "filters": [
    {
      "field": "region",
      "operator": "equals",
      "value": "UK"
    }
  ]
}
```

## MCP tools

Generate MCP tool metadata for metrics, for example:

```text
query_metric
list_metrics
get_metric_definition
```

or one tool per metric if that matches current SORX style better.

## Risk and policy

Metric queries are read-only but may expose sensitive data when dimensions or filters include PII.

Respect existing risk/policy machinery.

## Acceptance criteria

- SORX lists metrics through HTTP.
- SORX can query a metric through HTTP.
- SORX exposes MCP metadata for metrics.
- Sensitive dimensions/fields are considered in policy checks.
- Tests cover list/query/error paths.
