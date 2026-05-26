# SORX Metrics Runtime

SORX can load optional SoRLa metric metadata from a `.gtpack` entry:

```text
assets/sorla/metrics.json
```

The supported schema is `greentic.sorla.metrics.v1`. Packs without metrics continue to work.

## Pack Validation

`greentic-sorx doctor pack.gtpack` validates metric metadata when present:

- metric names must be unique;
- each metric must define exactly one of `measure` or `formula`;
- aggregate metrics require a `source`;
- supported aggregates are `count`, `sum`, `avg`, `min`, `max`, and `distinct_count`;
- supported time grains are `minute`, `hour`, `day`, `week`, and `month`;
- formula dependencies must exist and dependency cycles are rejected.

`greentic-sorx inspect pack.gtpack` includes a metrics summary with presence, count, metric names, and required capabilities.

## HTTP Routes

SORX exposes declared metrics on SORX-owned routes:

```text
GET  /v1/sorx/metrics
GET  /v1/sorx/metrics/{metric_name}
POST /v1/sorx/metrics/{metric_name}/query
```

Example query:

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

## MCP

Metric metadata is surfaced through deterministic SORX MCP tool metadata:

```text
sorx.metrics.list
sorx.metrics.get
sorx.metrics.query.<metric_name>
```

These are metadata tools for discovery; metric execution currently happens through the HTTP/runtime metric query path.

## Formulas

Formula metrics can reference named dependency metrics and use deterministic arithmetic:

```text
+
-
*
/
()
```

Arbitrary code execution is not supported.

## Audit, Policy, And Cache Hints

Metric list, definition-read, cache-miss, query, and policy-rejection events use the existing SORX audit sink.

Sensitive dimensions are denied by default in the metric HTTP query path. Cache hints from metric definitions are parsed and surfaced, but a full cache store is intentionally deferred.
