# PR 03 — Add deterministic ontology graph traversal commands and runtime service

## Repository

`greenticai/greentic-sorx`

## Objective

Add deterministic graph traversal over the ontology graph and, when configured, provider-backed relationship instances.

This is the bridge between static ontology metadata and runtime GraphRAG-style execution.

## Current repo alignment

This should depend on PR 01's loaded ontology structs rather than reparsing
archive entries in the CLI. Put reusable traversal logic in
`crates/greentic-sorx-core` or `crates/greentic-sorx-pack` only if it remains
pure/static; keep provider-backed runtime traversal behind a core trait.

The current CLI command surface lives in `crates/greentic-sorx-cli/src/lib.rs`
and already has `routes`, `mcp-tools`, `validate`, `deployments`, and `start`.
Add `graph` as a new top-level command without changing those outputs.

## CLI commands

Add:

```bash
greentic-sorx graph concepts pack.gtpack --json
greentic-sorx graph relationships pack.gtpack --json
greentic-sorx graph paths pack.gtpack --from Customer --to Contract --json
greentic-sorx graph neighbors pack.gtpack --entity-type Customer --entity-id c1 --depth 2 --json
greentic-sorx graph explain pack.gtpack --from Customer --to Contract --json
```

## Runtime service

Add a core service:

```rust
pub struct OntologyGraphService { ... }

impl OntologyGraphService {
    pub fn concept(&self, id: &str) -> Option<Concept>;
    pub fn relationships_from(&self, concept: &str) -> Vec<Relationship>;
    pub fn relationships_to(&self, concept: &str) -> Vec<Relationship>;
    pub fn find_type_paths(&self, from: &str, to: &str, max_depth: u8) -> Vec<TypePath>;
}
```

## Determinism

1. Sort concepts by ID.
2. Sort relationships by ID.
3. Sort paths lexically by relationship sequence.
4. Bound traversal depth.
5. Prevent cycles.
6. Output stable JSON.

## Provider-backed relationship traversal

If runtime provider bindings support relationship query/path find, add an internal seam but do not require live provider integration in this PR.

The current provider registry only stores `SorStoreProvider` CRUD providers, so
do not force relationship-instance traversal through that trait. Add a separate
optional trait or adapter boundary if needed.

## Tests

Add tests for:

- concept listing
- relationship listing
- type path depth 1
- type path depth 2+
- cycle handling
- unknown concept failure
- deterministic ordering
- CLI JSON output

## Docs

Add:

- `docs/ontology-graph.md`

## Acceptance criteria

```bash
cargo test --all-features
cargo test -p greentic-sorx-core ontology_graph --all-features
cargo test -p greentic-sorx-cli graph --all-features
bash ci/local_check.sh
```

Note: this repo does not currently have an `examples/` directory. Add or
generate ontology-enabled fixtures as part of the PR.
