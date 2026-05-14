# PR 02 — Add provider compatibility resolution for ontology-enabled packs

## Repository

`greenticai/greentic-sorx`

## Objective

Add deterministic compatibility checking between:

- ontology-enabled `.gtpack` artifacts
- startup answers
- configured providers
- provider pack/catalog metadata

Sorx should fail early when a pack requires ontology/evidence/entity-linking capabilities that no configured provider can satisfy.

## Current repo alignment

Build this on top of existing startup/provider pieces:

- `SorxRuntimeConfig.providers` and normalized answers in `crates/greentic-sorx-core/src/startup.rs`
- entity `BindingResolver` and `ProviderRegistry` in `crates/greentic-sorx-core/src/provider.rs`
- CLI dry-run output in `greentic-sorx start ... --dry-run --json`

The current provider layer is CRUD/store oriented. Keep ontology/evidence
capability resolution separate from entity store binding resolution so existing
runtime paths and tests continue to work.

## New model

Add:

- `ProviderCapabilityRequirement`
- `ResolvedProviderBinding`
- `ProviderCompatibilityReport`
- `ProviderCompatibilityIssue`
- `ProviderResolutionMode`

## Sources

Sorx should consider:

1. pack provider requirements
2. ontology provider requirements
3. retrieval bindings
4. startup answers
5. local provider registry
6. provider pack/catalog metadata where available

## Dry-run output

Extend:

```bash
greentic-sorx start pack.gtpack --answers answers.json --dry-run --json
```

with:

```json
{
  "provider_compatibility": {
    "status": "passed",
    "bindings": [
      {
        "requirement": "evidence.query",
        "provider_id": "greentic.sorla.provider.rag-mock",
        "capabilities": ["ontology-scoped-evidence-query", "entity-link"]
      }
    ],
    "issues": []
  }
}
```

## Failure modes

Use stable error categories:

```text
missing_provider
missing_capability
incompatible_contract_version
unsupported_ontology_schema
unsupported_retrieval_binding_schema
ambiguous_provider
```

## Tests

Add tests for:

- compatible provider passes
- missing evidence provider fails
- missing entity-link capability fails
- incompatible schema fails
- ambiguous provider reports deterministic issue
- dry-run JSON is stable
- existing entity provider binding behavior remains unchanged
- configured-provider validation mode still fails clearly until real configured adapters are wired

## Docs

Update:

- `docs/provider-bindings.md`
- `docs/answers.md`
- `docs/commands.md`

## Acceptance criteria

```bash
cargo test --all-features
cargo test -p greentic-sorx-core provider_compatibility --all-features
cargo test -p greentic-sorx-cli startup_cli --all-features
bash ci/local_check.sh
```

Note: this repo does not currently have an `examples/` directory. Reuse the
existing startup CLI fixture style or add a checked ontology-enabled fixture as
part of the PR.
