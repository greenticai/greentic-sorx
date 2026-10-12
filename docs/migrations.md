# Migrations

SORX applies schema/state migrations declared by a pack against its canonical
store at startup. Migrations are idempotent, namespace-scoped, and gated so an
unconfirmed breaking change cannot silently mutate a shared system-of-record.

## Where migrations come from

Migrations live in the pack's `assets/sorla/executable-contract.json` under a
top-level `"migrations"` array. The loader surfaces this JSON as
`LoadedSorlaPack.sorla_assets.executable_contract_json`. Each element
deserialises into a `CompatibilityMigration` (the SORX-local mirror of sorla's
`MigrationDecl` wire shape — kebab-case JSON):

```json
{
  "schema": "greentic.sorla.executable-contract.v1",
  "migrations": [
    {
      "name": "landlord-tenant-v2-fields",
      "compatibility": "additive",
      "idempotence_key": "landlord-tenant-v2-fields",
      "backfills": [
        { "record": "Tenant", "field": "date_of_birth", "default": null }
      ]
    }
  ]
}
```

A missing or `null` `migrations` field means "no migrations" (a clean no-op). A
present-but-malformed entry is a hard error — no entry is silently dropped.

## Classification

Each migration declares a `compatibility` mode:

| Mode                  | Auto-applied? | Notes                                            |
|-----------------------|---------------|--------------------------------------------------|
| `additive`            | yes           | New optional fields / backfills; always safe.    |
| `backward-compatible` | yes           | Backward-compatible reshape; applied on startup. |
| `breaking`            | only when confirmed | Refused unless explicitly confirmed.       |

Backfills query every record in a collection and set the target `field` to the
declared `default` wherever it is currently absent.

## Idempotence via the ledger

A migration is applied at most once per namespace. The applied set is persisted
in a durable **ledger**, keyed by the namespace prefix `sorx/<tenant>/<sor>`.
On every startup the runner loads the ledger, skips migrations already recorded
(`Skipped`), applies the rest (`Applied`), and records each newly-applied id.
Re-running the same pack is therefore a no-op.

### Ledger backends

- **`LocalMigrationLedger`** (dev / single-node) — a JSON file holding a
  per-namespace map (`{ "sorx/<tenant>/<sor>": { applied: [...] } }`), so a
  single shared file keeps tenants from colliding.
- **`FoundationDbStore`** (production, `--features foundationdb`) — records each
  applied id under the keyspace `<namespace-prefix>/migrations/<migration-id>`;
  a range scan reconstructs the applied set. Durable across reconnects.

Both implement the `MigrationLedger` trait:

```rust
pub trait MigrationLedger {
    fn load(&self, namespace: &ProviderNamespace) -> SorxResult<AppliedMigrations>;
    fn record_applied(&self, namespace: &ProviderNamespace, migration_id: &str) -> SorxResult<()>;
}
```

## The activation gate

Before applying anything, the runner consults
`evaluate_pending_migrations(state_mode, pending, applied, confirm_breaking)`.
When the deployment's `state_mode` is `SharedRequiresMigration` and a pending
(not-yet-applied) migration is `breaking` while `confirm_breaking` is `false`,
the gate returns an error (`migration_breaking_unconfirmed`) and nothing is
applied. Additive/backward-compatible migrations never block. Other state modes
do not trip the gate.

## Orchestration (reusable core)

`apply_pending_migrations` ties the gate, the runner, and the ledger together:

```rust
pub fn apply_pending_migrations(
    state_mode: StateMode,
    migrations: &[CompatibilityMigration],
    store: &dyn SorxCanonicalStore,
    ledger: &dyn MigrationLedger,
    namespace: &ProviderNamespace,
    confirm_breaking: bool,
) -> SorxResult<Vec<MigrationOutcome>>;
```

It loads the applied set, runs the gate (returning `Err` on unconfirmed
breaking), then applies each migration in order, recording every `Applied`
outcome in the ledger. It is backend-agnostic and fully unit-tested against the
in-memory store + a local-file ledger.

## Runtime wiring (`gtc start`)

At the local-start pack-load point (`HttpRuntime::from_pack_with_runtime_config`),
after the canonical store is built and before serving, SORX:

1. parses migrations from the pack's `executable-contract.json` (no-op when
   absent/empty);
2. resolves the namespace from `config.deployment` (`tenant_id` / `sor_name`)
   and the canonical store from the default provider binding;
3. uses a `LocalMigrationLedger` at `SORX_MIGRATION_LEDGER_PATH` (mirrors
   `SORX_REGISTRY_PATH`). If the pack ships migrations but no ledger path is
   configured, application is **skipped with a logged note** — without a durable
   ledger, idempotence across restarts cannot be guaranteed;
4. calls `apply_pending_migrations` with `confirm_breaking` from the
   `SORX_CONFIRM_BREAKING_MIGRATIONS` env flag (default `false`). An unconfirmed
   breaking migration **fails startup** rather than serving a half-migrated SoR.

The local-start config carries `deployment_mode`, not a registry `StateMode`, so
the wiring uses `StateMode::SharedRequiresMigration` to keep the gate active.

### Deferred to a later PR

A FoundationDB-backed ledger is not yet wired into `gtc start`: the provider
registry hands back an `Arc<dyn SorxCanonicalStore>` (a
`FoundationDbProviderAdapter`) whose inner `FoundationDbStore` — which already
implements `MigrationLedger` under the `foundationdb` feature — is not exposed.
Wiring it needs an accessor (or downcast) on the adapter; that, plus real-FDB +
real-pack end-to-end coverage, is the remaining hook.
