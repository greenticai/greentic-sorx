# Provider Bindings

Provider answers define runtime provider instances:

```json
{
  "providers": {
    "store": {
      "kind": "memory",
      "config_ref": "providers.memory.local"
    }
  }
}
```

Entity bindings map SoRLa entities to provider IDs and collections:

```json
{
  "bindings": {
    "entities": {
      "Tenant": {
        "provider": "store",
        "collection": "tenants"
      }
    }
  }
}
```

If bindings are omitted, SORX derives defaults from `agent-gateway.json`
entity and collection metadata. Explicit bindings must cover invoked entities.

Ontology-enabled packs may also require provider capabilities for evidence
retrieval or entity linking. Provider answer entries can declare deterministic
capability metadata:

```json
{
  "providers": {
    "rag": {
      "kind": "memory",
      "capabilities": ["ontology-scoped-evidence-query", "entity-link"],
      "contract_version": "greentic.sorx.provider.v1"
    }
  }
}
```

`start --dry-run --json` includes a `provider_compatibility` report. It reports
stable issue categories such as `missing_provider`, `missing_capability`,
`incompatible_contract_version`, `unsupported_ontology_schema`,
`unsupported_retrieval_binding_schema`, and `ambiguous_provider`.

Supported provider status:

- `memory`: implemented for local and CI execution.
- `foundationdb`: recognized, but currently returns a clear unavailable error
  until `greentic-sorla-providers` exposes a SORX-compatible store contract.
