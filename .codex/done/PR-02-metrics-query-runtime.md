# PR-02 — Add SORX metric query runtime model

Repository: `greenticai/greentic-sorx`

## Goal

Add a runtime query model for evaluating declared metrics through providers.

This PR does not need full HTTP/MCP exposure yet. It creates the internal execution pathway.

## Current repo validation

There is no metric query runtime yet. Existing provider abstractions live in `crates/greentic-sorx-core/src/provider.rs`:

- `SorStoreProvider` for CRUD/query/delete.
- `SorxCanonicalStore` for append events, index queries, graph traversal, external refs, and evidence.
- Provider implementations under `crates/greentic-sorx-core/src/providers/`.

There is no dependency on `greentic-sorla-providers` in this workspace, so the first runtime boundary should be a Sorx-owned trait with fake-provider tests.

Design update:

- Put runtime metric query types in `greentic-sorx-core`, while keeping pack metadata types in `greentic-sorx-pack`.
- Avoid coupling the core runtime to pack-loader structs directly if that would create a crate dependency cycle. Convert loaded metric definitions into core runtime definitions at the CLI/runtime boundary.
- Prefer a trait name that fits the existing provider style, such as `MetricRuntimeProvider` or `SorxMetricProvider`, and register it separately from `ProviderRegistry` unless a clean extension point exists.
- Formula evaluation must be deterministic and not use script/eval facilities.
- Use fake provider data in unit tests; do not introduce a real external provider dependency in this PR.

## Runtime query API

Add internal structures for `MetricQuery`, `MetricQueryResult`, and `MetricResultRow`.

## Provider boundary

Add or call a provider trait capable of executing metric queries:

```rust
pub trait MetricRuntimeProvider {
    fn query_metric(
        &self,
        definition: &RuntimeMetric,
        query: MetricQuery,
    ) -> Result<MetricQueryResult, SorxError>;
}
```

If provider traits live in `greentic-sorla-providers`, define the runtime adapter boundary here and align with provider PRs.

## Formula metrics

For derived formula metrics:

- resolve dependencies
- query dependency metrics
- calculate deterministic formula result
- reject unsupported formulas
- detect cycles
- avoid arbitrary code execution

MVP formula support can be limited to basic arithmetic over named dependency metrics:

```text
+
-
*
/
()
```

## Aggregates

Support initial aggregates:

```text
count
sum
avg
min
max
distinct_count
```

Provider may report unsupported capabilities.

## Acceptance criteria

- SORX can build a `MetricQuery` for a declared metric.
- SORX can delegate aggregate metrics to a provider.
- SORX can resolve simple formula metrics from dependencies.
- Unsupported metrics return clear errors.
- Unit tests use fake provider data.
