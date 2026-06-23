//! Migration runner: applies backfills via the store, idempotent via an
//! applied-migration set, additive/backward auto-apply, breaking gated behind
//! explicit confirmation.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::SorxError;
use crate::migration::plan::{CompatibilityMigration, MigrationBackfill};
use crate::provider::{
    ProviderNamespace, QueryOp, SorxCanonicalStore, UpdateOp, default_collection_name,
};

/// Tracks which migrations have been applied.
///
/// `#[derive(Serialize, Deserialize)]` is present for Task 0.3.4 durable
/// persistence — not used in this task.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppliedMigrations {
    applied: BTreeSet<String>,
}

impl AppliedMigrations {
    /// Returns `true` if the migration with the given id has already been applied.
    pub fn contains(&self, id: &str) -> bool {
        self.applied.contains(id)
    }

    /// Records a migration as applied.
    ///
    /// Returns `false` if the migration was already present (no-op); `true` if
    /// it was newly inserted.
    pub fn record(&mut self, id: &str) -> bool {
        self.applied.insert(id.to_string())
    }
}

/// The outcome of running a single migration.
#[derive(Debug, PartialEq)]
pub enum MigrationOutcome {
    /// Migration was applied; `backfilled` is the number of records updated.
    Applied { id: String, backfilled: usize },
    /// Migration was already applied — skipped (idempotent).
    Skipped { id: String },
    /// Migration is breaking and `confirm_breaking` was `false` — refused, NOT recorded.
    RefusedBreaking { id: String },
}

/// Errors that can occur while running a migration.
#[derive(Debug)]
pub enum MigrationRunError {
    BackfillFailed(SorxError),
}

impl std::fmt::Display for MigrationRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationRunError::BackfillFailed(err) => {
                write!(f, "backfill failed: {err}")
            }
        }
    }
}

impl std::error::Error for MigrationRunError {}

/// Runs migrations against a store, gating breaking changes behind an explicit
/// confirmation flag and ensuring idempotency via an `AppliedMigrations` set.
pub struct MigrationRunner<'a> {
    applied: &'a mut AppliedMigrations,
    confirm_breaking: bool,
}

impl<'a> MigrationRunner<'a> {
    /// Creates a new runner.
    ///
    /// - `applied`: mutable reference to the applied-migration ledger.
    /// - `confirm_breaking`: when `true`, breaking migrations are applied;
    ///   when `false`, they are refused without being recorded.
    pub fn new(applied: &'a mut AppliedMigrations, confirm_breaking: bool) -> Self {
        Self {
            applied,
            confirm_breaking,
        }
    }

    /// Applies a single migration.
    ///
    /// 1. If already in the ledger → `Skipped`.
    /// 2. If breaking and `!confirm_breaking` → `RefusedBreaking` (NOT recorded).
    /// 3. Apply all backfills via the store.
    /// 4. Record in the ledger and return `Applied { backfilled }`.
    pub fn run_one(
        &mut self,
        migration: &CompatibilityMigration,
        store: &dyn SorxCanonicalStore,
        namespace: &ProviderNamespace,
    ) -> Result<MigrationOutcome, MigrationRunError> {
        let id = migration.idempotence_id().to_string();

        if self.applied.contains(&id) {
            return Ok(MigrationOutcome::Skipped { id });
        }

        if migration.is_breaking() && !self.confirm_breaking {
            return Ok(MigrationOutcome::RefusedBreaking { id });
        }

        let mut backfilled: usize = 0;
        for backfill in &migration.backfills {
            backfilled += apply_backfill(store, namespace, backfill)
                .map_err(MigrationRunError::BackfillFailed)?;
        }

        self.applied.record(&id);
        Ok(MigrationOutcome::Applied { id, backfilled })
    }
}

/// Queries all records in a backfill's collection and, for each record missing
/// the target field, updates it with the specified default value.
///
/// Returns the number of records that were updated.
fn apply_backfill(
    store: &dyn SorxCanonicalStore,
    namespace: &ProviderNamespace,
    backfill: &MigrationBackfill,
) -> Result<usize, SorxError> {
    let collection = default_collection_name(&backfill.record);

    let result = store.query(QueryOp {
        namespace: namespace.clone(),
        entity: backfill.record.clone(),
        collection: collection.clone(),
        filter: Value::Object(Map::new()),
        order_by: vec![],
    })?;

    let mut count = 0usize;
    for record in result.records {
        if record.data.get(&backfill.field).is_none() {
            store.update(UpdateOp {
                namespace: namespace.clone(),
                entity: backfill.record.clone(),
                collection: collection.clone(),
                id: record.id.clone(),
                patch: json!({ &backfill.field: &backfill.default }),
                unique_indexes: vec![],
            })?;
            count += 1;
        }
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::{AppliedMigrations, MigrationOutcome, MigrationRunner};
    use crate::migration::{CompatibilityMigration, CompatibilityMode, MigrationBackfill};
    use crate::providers::MemoryStoreProvider;
    use crate::{CreateOp, GetOp, ProviderNamespace, SorStoreProvider, UniqueConflictBehavior};

    fn ns() -> ProviderNamespace {
        ProviderNamespace {
            tenant_id: "t".into(),
            sor_name: "tenancy".into(),
        }
    }

    fn seed(s: &MemoryStoreProvider, n: &ProviderNamespace) {
        s.create(CreateOp {
            namespace: n.clone(),
            entity: "Tenant".into(),
            collection: "tenants".into(),
            input: serde_json::json!({"id":"ten-1","name":"Ada"}),
            idempotency_key: None,
            unique_indexes: vec![],
            unique_behavior: UniqueConflictBehavior::Reject,
        })
        .expect("seed");
    }

    fn additive() -> CompatibilityMigration {
        CompatibilityMigration {
            name: "v2".into(),
            compatibility: CompatibilityMode::Additive,
            from_version: None,
            to_version: None,
            projection_updates: vec![],
            backfills: vec![MigrationBackfill {
                record: "Tenant".into(),
                field: "date_of_birth".into(),
                default: serde_json::Value::Null,
            }],
            idempotence_key: Some("v2".into()),
            notes: None,
        }
    }

    #[test]
    fn additive_applies_once_and_backfills() {
        let s = MemoryStoreProvider::new();
        let n = ns();
        seed(&s, &n);
        let mut applied = AppliedMigrations::default();
        let mut r = MigrationRunner::new(&mut applied, false);
        let o = r.run_one(&additive(), &s, &n).expect("apply");
        assert!(matches!(o, MigrationOutcome::Applied { backfilled: 1, .. }));
        let got = s
            .get(GetOp {
                namespace: n,
                entity: "Tenant".into(),
                collection: "tenants".into(),
                id: "ten-1".into(),
            })
            .expect("get")
            .expect("rec");
        assert!(got.data.get("date_of_birth").is_some());
    }

    #[test]
    fn applying_twice_is_noop() {
        let s = MemoryStoreProvider::new();
        let n = ns();
        seed(&s, &n);
        let mut applied = AppliedMigrations::default();
        let mut r = MigrationRunner::new(&mut applied, false);
        r.run_one(&additive(), &s, &n).expect("first");
        assert!(matches!(
            r.run_one(&additive(), &s, &n).expect("second"),
            MigrationOutcome::Skipped { .. }
        ));
    }

    #[test]
    fn breaking_refused_without_confirmation() {
        let s = MemoryStoreProvider::new();
        let n = ns();
        let mut m = additive();
        m.name = "breaker".into();
        m.idempotence_key = Some("breaker".into());
        m.compatibility = CompatibilityMode::Breaking;
        let mut applied = AppliedMigrations::default();
        let mut r = MigrationRunner::new(&mut applied, false);
        assert!(matches!(
            r.run_one(&m, &s, &n).expect("run"),
            MigrationOutcome::RefusedBreaking { .. }
        ));
        assert!(!applied.contains("breaker"));
    }

    #[test]
    fn breaking_applies_with_confirmation() {
        let s = MemoryStoreProvider::new();
        let n = ns();
        seed(&s, &n);
        let mut m = additive();
        m.name = "breaker".into();
        m.idempotence_key = Some("breaker".into());
        m.compatibility = CompatibilityMode::Breaking;
        let mut applied = AppliedMigrations::default();
        let mut r = MigrationRunner::new(&mut applied, true);
        assert!(matches!(
            r.run_one(&m, &s, &n).expect("run"),
            MigrationOutcome::Applied { .. }
        ));
        assert!(applied.contains("breaker"));
    }

    #[test]
    fn applied_migrations_serde_round_trips() {
        let mut a = AppliedMigrations::default();
        a.record("v2");
        let bytes = serde_json::to_vec(&a).expect("ser");
        let back: AppliedMigrations = serde_json::from_slice(&bytes).expect("de");
        assert!(back.contains("v2"));
    }
}
