# PR: Add canonical SORX provider abstraction

Repo: `greenticai/greentic-sorx`

## Goal
Evolve the existing `SorStoreProvider` CRUD/query boundary into a canonical, ontology-backed store contract that supports shared state across multiple active deployments.

## Current code assumptions

- The provider boundary already exists as `SorStoreProvider` with `create`, `get`, `update`, `query` and `delete`.
- The in-memory provider already implements deterministic CRUD/query and currently namespaces records by tenant, pack name and pack version through `ProviderNamespace`.
- The FoundationDB adapter boundary already exists, but deliberately returns `provider_unavailable`.
- Deployment registry state modes are currently `isolated`, `shared_compatible` and `shared_requires_migration`; there is no `shared_canonical` enum value yet.
- Production/local enforcement should be based on the existing deployment/runtime `environment` fields and registry state mode, not on a new unrelated config surface.

## New abstraction

```rust
trait SorxCanonicalStore: SorStoreProvider {
    fn append_event(...);
    fn query_index(...);
    fn traverse(...);
    fn get_external_refs(...);
    fn get_evidence(...);
}
```

Keep the existing CRUD method names unless a rename is part of a small compatibility layer. The first implementation should make canonical identity explicit instead of introducing a second, parallel CRUD API.

## State mode
Production should converge on shared canonical state by using the existing registry shape:

```yaml
state_mode: shared_compatible
state_namespace: sorx/{tenant}/{sor}
```

`isolated` may exist only for local tests/previews and must be rejected for production environments. `shared_requires_migration` remains the intermediate state for deployments that need PR 11 migration work before sharing canonical state.

## Acceptance criteria

- Existing memory provider implements the canonical contract while preserving current CRUD/query behavior.
- Provider namespaces are SoR-scoped for canonical state; pack/version identity remains deployment metadata rather than the primary storage namespace.
- FoundationDB adapter boundary is ready to consume provider crate implementation.
- SORX refuses production deployments with `state_mode: isolated`.
- Docs explain why data is SoR-scoped, not bundle-scoped.
