use greentic_sorx_core::migration::{
    AppliedMigrations, CompatibilityMigration, MigrationOutcome, MigrationRunner,
};
use greentic_sorx_core::providers::MemoryStoreProvider;
use greentic_sorx_core::{
    CreateOp, GetOp, ProviderNamespace, SorStoreProvider, UniqueConflictBehavior,
};

fn ns() -> ProviderNamespace {
    ProviderNamespace {
        tenant_id: "landlord".into(),
        sor_name: "tenancy".into(),
    }
}

fn fixture() -> CompatibilityMigration {
    serde_json::from_str(include_str!("fixtures/landlord_v2_migration.json"))
        .expect("parse landlord v2")
}

fn seed_tenant(s: &MemoryStoreProvider, n: &ProviderNamespace, id: &str) {
    s.create(CreateOp {
        namespace: n.clone(),
        entity: "Tenant".into(),
        collection: "tenants".into(),
        input: serde_json::json!({"id": id, "name": "Ada"}),
        idempotency_key: None,
        unique_indexes: vec![],
        unique_behavior: UniqueConflictBehavior::Reject,
    })
    .expect("seed");
}

#[test]
fn landlord_v2_backfills_only_present_tenant_fields() {
    let s = MemoryStoreProvider::new();
    let n = ns();
    seed_tenant(&s, &n, "ten-1");
    let mut applied = AppliedMigrations::default();
    let mut r = MigrationRunner::new(&mut applied, false);
    let o = r.run_one(&fixture(), &s, &n).expect("apply");
    // Only the seeded Tenant collection has a record → 4 Tenant fields backfilled.
    // Unit/Tenancy collections are empty → 0 there. Total = 4.
    assert!(
        matches!(o, MigrationOutcome::Applied { backfilled: 4, .. }),
        "got {o:?}"
    );
    let got = s
        .get(GetOp {
            namespace: n,
            entity: "Tenant".into(),
            collection: "tenants".into(),
            id: "ten-1".into(),
        })
        .expect("get")
        .expect("rec");
    for f in [
        "date_of_birth",
        "emergency_contact_name",
        "emergency_contact_phone",
        "preferred_contact_method",
    ] {
        assert!(got.data.get(f).is_some(), "missing backfilled field {f}");
    }
}

#[test]
fn landlord_v2_is_idempotent() {
    let s = MemoryStoreProvider::new();
    let n = ns();
    seed_tenant(&s, &n, "ten-1");
    let mut applied = AppliedMigrations::default();
    let mut r = MigrationRunner::new(&mut applied, false);
    r.run_one(&fixture(), &s, &n).expect("first");
    assert!(matches!(
        r.run_one(&fixture(), &s, &n).expect("second"),
        MigrationOutcome::Skipped { .. }
    ));
}
