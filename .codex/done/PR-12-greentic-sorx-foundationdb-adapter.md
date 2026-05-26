# PR: Wire SORX to FoundationDB provider implementation

Repo: `greenticai/greentic-sorx`

## Goal
Replace the current `provider_unavailable` FoundationDB boundary with a real adapter using `greentic-sorla-providers` contracts.

## Current code assumptions

- `FoundationDbProviderAdapter` already accepts `config_ref` and local/test direct config.
- The old `provider_unavailable` boundary is no longer accurate. The adapter currently delegates to a persistent `MemoryStoreProvider` using a deterministic local JSON path derived from `config_ref`/cluster/database fields.
- The adapter implements both `SorStoreProvider` and `SorxCanonicalStore`, so CRUD, events, index queries, graph traversal, external refs, and evidence storage are available through the same compatibility shim.
- `greentic-sorla-providers` is referenced in startup trust/default concepts, but this workspace still does not depend on a real provider crate.
- Production direct-config rejection is already part of startup validation; keep that behavior.
- The adapter already implements the canonical store contract from PR 07, not only the CRUD/query trait.

## Design update

Do not replace a nonexistent all-operations `provider_unavailable` stub. The remaining design question is whether/when to replace the persistent-memory compatibility adapter with a real `greentic-sorla-providers` FoundationDB implementation.

Future work should:

- Preserve the current provider trait surface and tests while swapping the adapter internals.
- Keep local/test direct config support and production direct-config rejection.
- Keep persistence and canonical-store behavior covered across restart.
- Fail clearly if a real provider crate is requested but unavailable or incompatible.
- Consider renaming or de-emphasizing legacy helper names like `FoundationDbProviderAdapter::unavailable`, which now construct the local compatibility adapter.

## Configuration

```yaml
providers:
  store:
    kind: foundationdb
    config_ref: env://providers/foundationdb/main
    contract_version: greentic.sorx.provider.v1
```

Test/local may allow direct config, production must use `config_ref`.

## Acceptance criteria

- SORX can start with FoundationDB provider config.
- Entity writes persist across SORX restart.
- Event log persists across restart.
- Index and graph queries persist across restart.
- Secrets are not stored in gtpack.
- Missing or incompatible provider crate versions fail with a clear startup/provider compatibility error.
