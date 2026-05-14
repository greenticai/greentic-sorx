# PR 04 — Add hybrid ontology-scoped evidence execution

## Repository

`greenticai/greentic-sorx`

## Objective

Add a deterministic runtime path for combining:

- ontology graph scope
- provider-backed evidence retrieval
- entity linking
- permission/policy context
- provenance

This is the production path toward ontology-driven GraphRAG.

## Current repo alignment

Build on:

- PR 01 ontology loading/validation
- PR 02 provider capability resolution
- PR 03 graph traversal
- existing runtime/policy/audit foundation in `crates/greentic-sorx-core`

The current runtime provider trait is `SorStoreProvider` for CRUD records only.
Introduce an evidence-provider boundary instead of overloading store providers.
Register test/memory evidence providers explicitly for deterministic tests.

## New command

Add:

```bash
greentic-sorx evidence query pack.gtpack   --answers answers.json   --query "find evidence relevant to this entity"   --entity-type Customer   --entity-id customer-123   --max-depth 2   --json
```

## Runtime behavior

1. Load ontology graph.
2. Validate requested entity type exists.
3. Build `OntologyScope`.
4. Resolve retrieval bindings.
5. Resolve evidence provider.
6. Query evidence provider using generic `EvidenceQueryFilter`.
7. Optionally invoke entity-link provider.
8. Return evidence with linked entities, source refs, provenance, and policy context.
9. Do not call an LLM in this PR.

## Output shape

```json
{
  "schema": "greentic.sorx.evidence-query-result.v1",
  "query": "...",
  "ontology_scope": {
    "root_entities": []
  },
  "evidence": [
    {
      "evidence_id": "...",
      "source_ref": "...",
      "snippet": "...",
      "score": 0.91,
      "linked_entities": [],
      "provenance": "..."
    }
  ],
  "explain": {
    "retrieval_binding": "...",
    "provider_id": "...",
    "graph_paths_considered": []
  }
}
```

## Tests

Add tests for:

- query with entity scope
- query with relationship traversal
- missing retrieval provider failure
- missing entity type failure
- deterministic output
- provenance present
- no LLM dependency
- existing `start`, `routes`, `mcp`, and endpoint runtime tests keep passing
- evidence provider behavior is covered with deterministic in-memory/mock providers

## Docs

Add:

- `docs/evidence.md`
- update `docs/commands.md`

## Acceptance criteria

```bash
cargo test --all-features
cargo test -p greentic-sorx-core evidence --all-features
cargo test -p greentic-sorx-cli evidence --all-features
bash ci/local_check.sh
```

Note: this repo does not currently have an `examples/` directory. Add or
generate ontology/evidence fixtures as part of the PR.
