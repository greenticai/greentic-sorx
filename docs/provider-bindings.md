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

Supported provider status:

- `memory`: implemented for local and CI execution.
- `foundationdb`: recognized, but currently returns a clear unavailable error
  until `greentic-sorla-providers` exposes a SORX-compatible store contract.
