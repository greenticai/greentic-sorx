# PR: Wire SORX to FoundationDB provider implementation

Repo: `greenticai/greentic-sorx`

## Goal
Replace the current `provider_unavailable` FoundationDB boundary with a real adapter using `greentic-sorla-providers` contracts.

## Current code assumptions

- `FoundationDbProviderAdapter` already accepts `config_ref` and local/test direct config, then returns `provider_unavailable` for all `SorStoreProvider` operations.
- `greentic-sorla-providers` is referenced in startup trust defaults, but this workspace does not currently depend on a provider crate.
- Production direct-config rejection is already part of startup validation; keep that behavior.
- The adapter should implement the canonical store contract from PR 07, not only the current CRUD/query trait.

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
