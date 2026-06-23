//! Gated integration test for the FoundationDB migration ledger methods.
//!
//! Requires a live FDB cluster via `FDB_CLUSTER_FILE` (see
//! `tests/foundationdb_real.rs` for the full harness context). Compiled and
//! run only with `--features foundationdb`.
#![cfg(feature = "foundationdb")]

use greentic_sorx_core::ProviderNamespace;
use greentic_sorx_core::providers::foundationdb_real::FoundationDbStore;

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
