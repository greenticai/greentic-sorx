//! Migration orchestration: the reusable core that ties the activation gate,
//! the [`MigrationRunner`], and a durable ledger together.
//!
//! [`apply_pending_migrations`] is the single entry point the runtime calls at
//! pack-load time. It is namespace-scoped, idempotent across runs (via the
//! [`MigrationLedger`]), auto-applies additive/backward-compatible migrations,
//! and refuses an unconfirmed breaking migration in a
//! `SharedRequiresMigration` deployment (returning `Err`).

use crate::deployment::{StateMode, evaluate_pending_migrations};
use crate::error::{SorxError, SorxResult};
use crate::migration::plan::CompatibilityMigration;
use crate::migration::runner::{AppliedMigrations, MigrationOutcome, MigrationRunner};
use crate::provider::{ProviderNamespace, SorxCanonicalStore};

/// Durable, namespace-scoped record of which migrations have been applied.
///
/// Backends implement load/record against their own storage. The trait keeps
/// the orchestration logic agnostic of the backend (local JSON file in
/// dev/single-node, FoundationDB subspace in production).
pub trait MigrationLedger {
    /// Load the applied-migration set recorded for `namespace`.
    ///
    /// A namespace with no recorded migrations returns an empty
    /// [`AppliedMigrations`] rather than an error.
    fn load(&self, namespace: &ProviderNamespace) -> SorxResult<AppliedMigrations>;

    /// Durably record `migration_id` as applied under `namespace`.
    ///
    /// Implementations must be idempotent: recording the same id twice is safe.
    fn record_applied(&self, namespace: &ProviderNamespace, migration_id: &str) -> SorxResult<()>;
}

/// Apply pending pack migrations against `store`, recording each newly-applied
/// migration in `ledger`.
///
/// Behaviour:
/// - Loads the applied set for `namespace` from `ledger`.
/// - Runs the activation gate ([`evaluate_pending_migrations`]); if a pending
///   breaking migration is unconfirmed in a `SharedRequiresMigration`
///   deployment, returns `Err` with code `migration_breaking_unconfirmed`
///   (nothing is applied).
/// - Otherwise auto-applies additive/backward-compatible migrations (and
///   breaking ones when `confirm_breaking`), recording each `Applied` migration
///   in the ledger so subsequent runs report `Skipped`.
///
/// Returns the per-migration outcomes in input order.
pub fn apply_pending_migrations(
    state_mode: StateMode,
    migrations: &[CompatibilityMigration],
    store: &dyn SorxCanonicalStore,
    ledger: &dyn MigrationLedger,
    namespace: &ProviderNamespace,
    confirm_breaking: bool,
) -> SorxResult<Vec<MigrationOutcome>> {
    let mut applied = ledger.load(namespace)?;

    // Pre-flight gate: refuse before mutating any state if an unconfirmed
    // breaking migration is pending under a migration-requiring deployment.
    evaluate_pending_migrations(state_mode, migrations, &applied, confirm_breaking)
        .map_err(|err| SorxError::new("migration_breaking_unconfirmed", err.message))?;

    let mut outcomes = Vec::with_capacity(migrations.len());
    let mut runner = MigrationRunner::new(&mut applied, confirm_breaking);
    for migration in migrations {
        let outcome = runner
            .run_one(migration, store, namespace)
            .map_err(|err| SorxError::new("migration_run_failed", err.to_string()))?;
        if let MigrationOutcome::Applied { id, .. } = &outcome {
            ledger.record_applied(namespace, id)?;
        }
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::ledger::LocalMigrationLedger;
    use crate::migration::plan::{CompatibilityMode, MigrationBackfill};
    use crate::providers::MemoryStoreProvider;
    use crate::{CreateOp, ProviderNamespace, SorStoreProvider, UniqueConflictBehavior};

    fn ns() -> ProviderNamespace {
        ProviderNamespace {
            tenant_id: "acme".into(),
            sor_name: "tenancy".into(),
        }
    }

    fn seed(store: &MemoryStoreProvider, namespace: &ProviderNamespace) {
        store
            .create(CreateOp {
                namespace: namespace.clone(),
                entity: "Tenant".into(),
                collection: "tenants".into(),
                input: serde_json::json!({"id": "ten-1", "name": "Ada"}),
                idempotency_key: None,
                unique_indexes: vec![],
                unique_behavior: UniqueConflictBehavior::Reject,
            })
            .expect("seed tenant");
    }

    fn additive() -> CompatibilityMigration {
        CompatibilityMigration {
            name: "landlord-tenant-v2-fields".into(),
            compatibility: CompatibilityMode::Additive,
            from_version: None,
            to_version: None,
            projection_updates: vec![],
            backfills: vec![MigrationBackfill {
                record: "Tenant".into(),
                field: "date_of_birth".into(),
                default: serde_json::Value::Null,
            }],
            idempotence_key: Some("landlord-tenant-v2-fields".into()),
            notes: None,
        }
    }

    fn breaking() -> CompatibilityMigration {
        CompatibilityMigration {
            name: "split-name".into(),
            compatibility: CompatibilityMode::Breaking,
            from_version: None,
            to_version: None,
            projection_updates: vec![],
            backfills: vec![],
            idempotence_key: Some("split-name".into()),
            notes: None,
        }
    }

    #[test]
    fn applies_then_idempotent_skip_across_runs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = LocalMigrationLedger::new(dir.path().join("ledger.json"));
        let store = MemoryStoreProvider::new();
        let namespace = ns();
        seed(&store, &namespace);

        // First run applies and records.
        let first = apply_pending_migrations(
            StateMode::SharedRequiresMigration,
            &[additive()],
            &store,
            &ledger,
            &namespace,
            false,
        )
        .expect("first run");
        assert_eq!(first.len(), 1);
        assert!(
            matches!(first[0], MigrationOutcome::Applied { backfilled: 1, .. }),
            "expected Applied with one backfill, got {:?}",
            first[0]
        );

        // Second run sees the persisted ledger and skips.
        let second = apply_pending_migrations(
            StateMode::SharedRequiresMigration,
            &[additive()],
            &store,
            &ledger,
            &namespace,
            false,
        )
        .expect("second run");
        assert!(
            matches!(second[0], MigrationOutcome::Skipped { .. }),
            "expected Skipped on re-run, got {:?}",
            second[0]
        );
    }

    #[test]
    fn unconfirmed_breaking_requires_migration_returns_err() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = LocalMigrationLedger::new(dir.path().join("ledger.json"));
        let store = MemoryStoreProvider::new();
        let namespace = ns();

        let err = apply_pending_migrations(
            StateMode::SharedRequiresMigration,
            &[breaking()],
            &store,
            &ledger,
            &namespace,
            false,
        )
        .expect_err("unconfirmed breaking must fail");
        assert_eq!(err.code, "migration_breaking_unconfirmed");
        // Nothing recorded.
        let applied = MigrationLedger::load(&ledger, &namespace).expect("load");
        assert!(!applied.contains("split-name"));
    }

    #[test]
    fn confirmed_breaking_applies_and_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = LocalMigrationLedger::new(dir.path().join("ledger.json"));
        let store = MemoryStoreProvider::new();
        let namespace = ns();

        let outcomes = apply_pending_migrations(
            StateMode::SharedRequiresMigration,
            &[breaking()],
            &store,
            &ledger,
            &namespace,
            true,
        )
        .expect("confirmed breaking applies");
        assert!(matches!(outcomes[0], MigrationOutcome::Applied { .. }));
        assert!(
            MigrationLedger::load(&ledger, &namespace)
                .expect("load")
                .contains("split-name")
        );
    }

    #[test]
    fn empty_migrations_is_noop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = LocalMigrationLedger::new(dir.path().join("ledger.json"));
        let store = MemoryStoreProvider::new();
        let outcomes = apply_pending_migrations(
            StateMode::SharedRequiresMigration,
            &[],
            &store,
            &ledger,
            &ns(),
            false,
        )
        .expect("empty is ok");
        assert!(outcomes.is_empty());
    }

    #[test]
    fn isolated_mode_skips_gate_but_still_applies_additive() {
        // Non-migration-requiring modes don't trip the gate; additive still runs.
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = LocalMigrationLedger::new(dir.path().join("ledger.json"));
        let store = MemoryStoreProvider::new();
        let namespace = ns();
        seed(&store, &namespace);
        let outcomes = apply_pending_migrations(
            StateMode::Isolated,
            &[additive()],
            &store,
            &ledger,
            &namespace,
            false,
        )
        .expect("isolated additive");
        assert!(matches!(outcomes[0], MigrationOutcome::Applied { .. }));
    }

    #[test]
    fn ledger_is_namespace_scoped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = LocalMigrationLedger::new(dir.path().join("ledger.json"));
        let store = MemoryStoreProvider::new();
        let ns_a = ProviderNamespace {
            tenant_id: "tenant-a".into(),
            sor_name: "tenancy".into(),
        };
        let ns_b = ProviderNamespace {
            tenant_id: "tenant-b".into(),
            sor_name: "tenancy".into(),
        };
        seed(&store, &ns_a);
        seed(&store, &ns_b);

        apply_pending_migrations(
            StateMode::SharedRequiresMigration,
            &[additive()],
            &store,
            &ledger,
            &ns_a,
            false,
        )
        .expect("apply ns_a");

        // ns_b shares the same on-disk file but must NOT inherit ns_a's record.
        let outcomes = apply_pending_migrations(
            StateMode::SharedRequiresMigration,
            &[additive()],
            &store,
            &ledger,
            &ns_b,
            false,
        )
        .expect("apply ns_b");
        assert!(
            matches!(outcomes[0], MigrationOutcome::Applied { .. }),
            "tenant-b must apply independently, got {:?}",
            outcomes[0]
        );
    }
}
