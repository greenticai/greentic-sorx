# PR 08 — Provider Binding Model and FoundationDB Provider Integration

## Goal

Add real provider binding resolution and integrate the FoundationDB-backed SoRLa provider if available.

If the FoundationDB provider is not ready or not in this repo, add a clean adapter boundary and keep in-memory as default for CI.

## Provider binding config

Startup answers should support:

```json
{
  "providers": {
    "store": {
      "kind": "foundationdb",
      "config_ref": "providers.foundationdb.local"
    }
  },
  "bindings": {
    "entities": {
      "Landlord": {
        "provider": "store",
        "collection": "landlords"
      },
      "Tenant": {
        "provider": "store",
        "collection": "tenants"
      },
      "Property": {
        "provider": "store",
        "collection": "properties"
      },
      "Unit": {
        "provider": "store",
        "collection": "units"
      },
      "Tenancy": {
        "provider": "store",
        "collection": "tenancies"
      },
      "Payment": {
        "provider": "store",
        "collection": "payments"
      },
      "MaintenanceRequest": {
        "provider": "store",
        "collection": "maintenance_requests"
      }
    }
  }
}
```

If bindings are omitted, derive safe defaults from entity names where possible.

## Provider resolution

Implement:

```rust
pub enum StoreProviderKind {
    Memory,
    FoundationDb,
    External(String),
}

pub struct ProviderBinding {
    pub entity: String,
    pub provider_id: String,
    pub collection: String,
}

pub struct BindingResolver {
    pub entity_bindings: HashMap<String, ProviderBinding>,
}
```

## Credential/config boundary

Sorx must not put secrets into `.gtpack`.

`config_ref` should point to external configuration resolved at runtime.

For local dev:

```json
{
  "providers": {
    "store": {
      "kind": "foundationdb",
      "config": {
        "cluster_file": "./.local/fdb.cluster",
        "database": "DB"
      }
    }
  }
}
```

For normal mode:

```json
{
  "providers": {
    "store": {
      "kind": "foundationdb",
      "config_ref": "providers.foundationdb.local"
    }
  }
}
```

Prefer `config_ref` and document direct `config` as local/test only.

## FoundationDB integration

Audit `greentic-sorla-providers` for the FoundationDB provider.

If usable:

- add dependency/adapter
- map Sorx `CreateOp/GetOp/UpdateOp/QueryOp/DeleteOp` to provider calls
- support namespaces by tenant and pack/version
- add integration tests behind feature flag

If not usable:

- add `FoundationDbProviderAdapter` stub returning clear error
- document required provider trait alignment
- add a follow-up issue/PR plan

## Storage layout guidance

Use tenant-aware keys.

Example conceptual layout:

```text
/sorx/{tenant_id}/{pack_name}/{pack_version}/{entity}/{id}
/sorx/{tenant_id}/{pack_name}/{pack_version}/indexes/{entity}/{field}/{value}/{id}
```

Do not lock this in if provider already has a better layout.

## Tests

Add tests:

- entity binding resolves provider and collection
- missing binding fails clearly
- default binding works for simple entity names
- provider kind `memory` works
- provider kind `foundationdb` either works under feature flag or fails with clear unavailable error
- config_ref accepted without exposing secret values
- direct config only allowed in local/test mode
- tenant namespace is applied to provider operations

Optional integration test:

```bash
cargo test --features foundationdb-integration
```

## CI

Do not require FoundationDB for normal PR checks unless already available.

Add manual or optional workflow support:

```yaml
workflow_dispatch:
  inputs:
    provider:
      default: foundationdb
```

## Acceptance criteria

- Provider bindings are explicit and validated.
- Runtime resolves entity operations to providers.
- Memory provider still works.
- FoundationDB integration is added if available, otherwise adapter gap is documented.
- Secrets remain outside `.gtpack`.
- Tests cover binding resolution.

## Codex working style

Complete as much as possible in one pass. Do not fake FoundationDB success. If the provider is not available, build the adapter boundary and document the exact gap.
