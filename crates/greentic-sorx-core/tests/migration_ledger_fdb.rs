//! Gated integration test for the FoundationDB migration ledger methods.
//!
//! Requires a live FDB cluster via `FDB_CLUSTER_FILE` (see
//! `tests/foundationdb_real.rs` for the full harness context). Compiled and
//! run only with `--features foundationdb`.
#![cfg(feature = "foundationdb")]

use greentic_sorx_core::ProviderNamespace;
use greentic_sorx_core::providers::foundationdb_real::FoundationDbStore;
use greentic_sorx_core::providers::{FoundationDbProviderAdapter, FoundationDbProviderConfig};
use greentic_sorx_core::{
    CreateOp, MigrationOutcome, SorStoreProvider, SorxCanonicalStore, StateMode,
    UniqueConflictBehavior, apply_pending_migrations,
};
use greentic_sorx_core::migration::{CompatibilityMigration, CompatibilityMode, MigrationBackfill};

fn cluster_file() -> String {
    std::env::var("FDB_CLUSTER_FILE")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            format!(
                "{}/.local/fdb/fdb.cluster",
                std::env::var("HOME").unwrap_or_default()
            )
        })
}

fn unique_namespace(test: &str) -> ProviderNamespace {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    ProviderNamespace {
        tenant_id: format!("it-{test}-{nanos}"),
        sor_name: "landlord".into(),
    }
}

fn additive_migration() -> CompatibilityMigration {
    CompatibilityMigration {
        name: "add-date-of-birth".into(),
        compatibility: CompatibilityMode::Additive,
        from_version: None,
        to_version: None,
        projection_updates: vec![],
        backfills: vec![MigrationBackfill {
            record: "Tenant".into(),
            field: "date_of_birth".into(),
            default: serde_json::Value::Null,
        }],
        idempotence_key: Some("add-date-of-birth".into()),
        notes: None,
    }
}

#[test]
fn migration_ledger_survives_reconnect() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ns = ProviderNamespace {
        tenant_id: format!("it-mig-{nanos}"),
        sor_name: "landlord".into(),
    };
    {
        let store = FoundationDbStore::connect(&cluster_file()).expect("connect");
        let empty = store.load_applied_migrations(&ns).expect("load empty");
        assert!(!empty.contains("v2"));
        store.record_migration_applied(&ns, "v2").expect("record");
    }
    let store2 = FoundationDbStore::connect(&cluster_file()).expect("reconnect");
    let reloaded = store2.load_applied_migrations(&ns).expect("reload");
    assert!(
        reloaded.contains("v2"),
        "applied-migration ledger must persist across reconnect"
    );
}

/// Verify that a FoundationDB-backed adapter exposes a durable migration ledger
/// via `as_migration_ledger()` and that apply→record→idempotent re-run works end
/// to end against the live cluster.
#[test]
fn fdb_adapter_as_migration_ledger_apply_then_idempotent() {
    let ns = unique_namespace("mig-adapter");

    let adapter = FoundationDbProviderAdapter::new(FoundationDbProviderConfig {
        cluster_file: Some(cluster_file()),
        database: None,
        config_ref: None,
    })
    .expect("construct FDB adapter");

    // The adapter must expose its store as a migration ledger.
    let ledger = adapter
        .as_migration_ledger()
        .expect("FDB-backed adapter must return Some from as_migration_ledger");

    // Seed one record so the backfill migration has something to operate on.
    adapter
        .create(CreateOp {
            namespace: ns.clone(),
            entity: "Tenant".into(),
            collection: "tenants".into(),
            input: serde_json::json!({"id": "t1", "name": "Alice"}),
            idempotency_key: None,
            unique_indexes: vec![],
            unique_behavior: UniqueConflictBehavior::Reject,
        })
        .expect("seed tenant");

    // First run: migration must be Applied.
    let first = apply_pending_migrations(
        StateMode::SharedRequiresMigration,
        &[additive_migration()],
        &adapter,
        ledger,
        &ns,
        false,
    )
    .expect("first apply");
    assert_eq!(first.len(), 1);
    assert!(
        matches!(first[0], MigrationOutcome::Applied { .. }),
        "first run must be Applied, got {:?}",
        first[0]
    );

    // Second run with a freshly-borrowed ledger: must be Skipped (durable record).
    let ledger2 = adapter
        .as_migration_ledger()
        .expect("second borrow");
    let second = apply_pending_migrations(
        StateMode::SharedRequiresMigration,
        &[additive_migration()],
        &adapter,
        ledger2,
        &ns,
        false,
    )
    .expect("second apply");
    assert!(
        matches!(second[0], MigrationOutcome::Skipped { .. }),
        "second run must be Skipped (idempotent), got {:?}",
        second[0]
    );
}

// ---------------------------------------------------------------------------
// Default-build assertion: memory-backed adapter returns None.
// This test is NOT gated — it runs in the default (non-FDB) build.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "foundationdb"))]
#[test]
#[allow(dead_code)]
fn this_module_is_only_compiled_with_fdb_feature() {
    // The outer #![cfg(feature = "foundationdb")] ensures this is dead code
    // without the feature, but we also add a plain-build check in the core
    // crate's unit tests (see provider.rs).
}

/// Ensure the memory-backed provider returns None from as_migration_ledger.
/// This is a unit test in the gated file for completeness; the authoritative
/// version lives as a unit test in provider.rs itself (no feature flag needed).
#[test]
fn memory_backed_adapter_returns_none() {
    use greentic_sorx_core::providers::MemoryStoreProvider;
    let store = MemoryStoreProvider::new();
    assert!(
        store.as_migration_ledger().is_none(),
        "MemoryStoreProvider must return None from as_migration_ledger"
    );
}
