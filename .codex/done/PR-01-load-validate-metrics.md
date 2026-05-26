# PR-01 — Load and validate SoRLa metrics metadata in SORX

Repository: `greenticai/greentic-sorx`

## Goal

Teach SORX to load metrics metadata from a `.gtpack`, validate it, and include it in runtime inspection.

## Current repo validation

The repo does not currently contain a metrics model or loader. `rg metrics` only finds the new PR notes, so this is valid new feature work.

The pack loader shape to extend is:

- `crates/greentic-sorx-pack/src/loader.rs`
- `LoadedSorlaPack`
- `SorlaAssets`
- `inspect_loaded_sorla_pack`
- `doctor_sorla_loaded_pack`
- validation helpers alongside ontology/business-action validation

Current optional SoRLa assets include ontology, retrieval bindings, business actions, MCP tools, OpenAPI overlay, Arazzo, and llms fragment. Metrics should follow that optional-asset pattern. Packs without `assets/sorla/metrics.json` must continue to load and pass existing tests.

Design update:

- Add metrics support in `greentic-sorx-pack` first, not in `greentic-sorx-core`.
- Add a dedicated `metrics` module in `greentic-sorx-pack` similar to `business_actions.rs` or `ontology.rs`.
- Extend `SorlaAssets` with `metrics: Option<MetricAssets>` and extend `SorxInspectSorla` or add a nested inspect summary type. This will change public inspect JSON, so keep absent metrics represented deterministically.
- Doctor diagnostics should be accumulated through `LoadedSorlaPack.doctor_errors`/`doctor_warnings`, matching existing validation style.
- Keep schema validation local and deterministic; provider capability checks can be metadata-only in this PR because no metric runtime provider exists yet.

## Input artifact

Load:

```text
assets/sorla/metrics.json
```

Expected schema:

```json
{
  "schema": "greentic.sorla.metrics.v1",
  "package": {
    "name": "commerce-sor",
    "version": "0.1.0"
  },
  "metrics": []
}
```

## Runtime model

Add SORX runtime types for `RuntimeMetricCatalog`, `RuntimeMetric`, source, measure, filters, time, window, target and formula dependencies.

## Validation

SORX should validate:

- schema version supported
- metric names are unique
- required fields exist
- aggregate values are supported
- time grains are supported
- formula dependencies exist
- dependency cycles are rejected
- provider capability requirements can be checked against selected provider if available

## Doctor

Extend:

```bash
greentic-sorx doctor pack.gtpack
```

to include metrics diagnostics.

## Inspect

Extend:

```bash
greentic-sorx inspect pack.gtpack
```

to show metric names/counts and required capabilities.

## Acceptance criteria

- SORX loads metrics metadata from `.gtpack`.
- SORX validates metrics at doctor/start boundary.
- Inspect output includes metrics summary.
- Invalid metrics produce clear diagnostics.
- Packs without metrics continue to work.
