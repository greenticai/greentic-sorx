# PR 01 — Load and validate SoRLa ontology artifacts in `greentic-sorx`

## Repository

`greenticai/greentic-sorx`

## Objective

Teach Sorx to load, inspect, and validate ontology artifacts emitted by Sorla `.gtpack` archives.

Sorx should not generate ontology. It consumes the deterministic artifacts emitted by Sorla.

## Current repo alignment

Build this on top of the existing `greentic-sorx-pack` loader in
`crates/greentic-sorx-pack/src/loader.rs`.

The current loader already validates `.gtpack` archive paths, manifest
extension references, required SoRLa/SORX assets, MCP metadata, validation-suite
assets, and stable `doctor`/`inspect` reports.

Do not add a second pack reader. Extend `LoadedSorlaPack`, `SorlaAssets`,
`SorxInspectReport`, and the existing doctor warning/error flow.

## New support

Load:

```text
assets/sorla/ontology.graph.json
assets/sorla/ontology.ir.cbor
assets/sorla/retrieval-bindings.json
```

when present.

## Pack loader changes

Extend `greentic-sorx-pack` with:

- `OntologyGraph`
- `OntologyConcept`
- `OntologyRelationship`
- `RetrievalBindings`
- artifact discovery by manifest extension
- static validation helpers

## Doctor changes

`greentic-sorx doctor <pack.gtpack>` should validate:

1. ontology graph schema is supported
2. all concepts have unique IDs
3. all relationships have unique IDs
4. relationship endpoints exist
5. backing records exist if record metadata is available
6. retrieval bindings reference existing concepts/relationships
7. ontology IR hash matches if both graph and IR are present
8. no secret-like values in ontology and retrieval-binding payloads
9. no absolute local machine paths in ontology and retrieval-binding payloads

## Inspect changes

`greentic-sorx inspect <pack.gtpack> --json` should include:

```json
{
  "ontology": {
    "present": true,
    "schema": "greentic.sorla.ontology.graph.v1",
    "concept_count": 10,
    "relationship_count": 14,
    "retrieval_bindings_present": true
  }
}
```

## Tests

Add tests for:

- pack with no ontology still works
- pack with valid ontology passes doctor
- pack with invalid relationship fails doctor
- inspect emits stable ontology summary
- retrieval binding validation
- manifest extension references may point at ontology artifacts
- ontology artifacts are optional and backwards compatible with current packs

## Docs

Update:

- `docs/commands.md`
- `docs/validation-suites.md`
- `docs/getting-started.md`

## Acceptance criteria

```bash
cargo test --all-features
cargo test -p greentic-sorx-pack ontology --all-features
cargo test -p greentic-sorx-cli inspect --all-features
bash ci/local_check.sh
```

Note: this repo does not currently have an `examples/` directory. Use checked-in
test fixtures or generated temp `.gtpack` fixtures following the existing
`crates/greentic-sorx-pack/src/loader.rs` and CLI test patterns.
