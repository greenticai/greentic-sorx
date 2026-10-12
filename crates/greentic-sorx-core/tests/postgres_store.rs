//! Integration tests for the Postgres-backed canonical store.
//!
//! Compiled with `--features postgres`, and SKIPPED unless
//! `SORX_TEST_POSTGRES_URL` names a reachable database (CI provides one with a
//! `postgres` service container). The first nine tests are the FoundationDB
//! suite run against this backend, so the two stores are held to one contract;
//! the rest cover what only a multi-writer SQL store can get wrong.
//!
//! Each test uses a unique tenant slug so reruns never collide.
#![cfg(feature = "postgres")]

use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

use greentic_sorx_core::{
    AppendEventOp, CreateOp, DeleteOp, ExternalRefsOp, GetOp, PostgresStore, ProviderNamespace,
    QueryOp, QueryOrder, QueryOrderDirection, SorStoreProvider, SorxCanonicalStore,
    StoreEvidenceOp, UniqueConflictBehavior, UniqueIndex, UpdateOp,
};
use serde_json::json;

fn url() -> Option<String> {
    std::env::var("SORX_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Skip (return early) when no database is configured.
macro_rules! require_db {
    () => {
        if url().is_none() {
            eprintln!("SORX_TEST_POSTGRES_URL unset; skipping");
            return;
        }
    };
}

fn unique_namespace(test: &str) -> ProviderNamespace {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    ProviderNamespace {
        tenant_id: format!("it-{test}-{nanos}"),
        sor_name: "landlord".to_string(),
    }
}

fn store() -> PostgresStore {
    PostgresStore::connect_url(&url().expect("SORX_TEST_POSTGRES_URL"), 4)
        .expect("connect to Postgres")
}

#[test]
fn pg_crud_roundtrip() {
    require_db!();
    let store = store();
    let ns = unique_namespace("crud");

    // create
    let created = store
        .create(CreateOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            input: json!({"id": "tenant-1", "name": "Alice", "score": 10}),
            idempotency_key: None,
            unique_indexes: Vec::new(),
            unique_behavior: UniqueConflictBehavior::Reject,
        })
        .expect("create");
    assert_eq!(created.id, "tenant-1");
    assert_eq!(created.version, 1);

    // get
    let fetched = store
        .get(GetOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "tenant-1".to_string(),
        })
        .expect("get")
        .expect("present");
    assert_eq!(fetched, created);

    // second entity for ordering
    store
        .create(CreateOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            input: json!({"id": "tenant-2", "name": "Bob", "score": 5}),
            idempotency_key: None,
            unique_indexes: Vec::new(),
            unique_behavior: UniqueConflictBehavior::Reject,
        })
        .expect("create 2");

    // update (patch) -> version increments
    let updated = store
        .update(UpdateOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "tenant-1".to_string(),
            patch: json!({"score": 99}),
            unique_indexes: Vec::new(),
        })
        .expect("update");
    assert_eq!(updated.version, 2);
    assert_eq!(updated.data.get("score"), Some(&json!(99)));
    assert_eq!(updated.data.get("name"), Some(&json!("Alice")));

    // query (filter + order desc by score)
    let result = store
        .query(QueryOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            filter: json!({}),
            order_by: vec![QueryOrder {
                field: "score".to_string(),
                direction: QueryOrderDirection::Desc,
            }],
        })
        .expect("query");
    assert_eq!(result.records.len(), 2);
    assert_eq!(result.records[0].id, "tenant-1"); // score 99
    assert_eq!(result.records[1].id, "tenant-2"); // score 5

    // filtered query
    let filtered = store
        .query(QueryOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            filter: json!({"name": "Bob"}),
            order_by: Vec::new(),
        })
        .expect("query filtered");
    assert_eq!(filtered.records.len(), 1);
    assert_eq!(filtered.records[0].id, "tenant-2");

    // delete
    let deleted = store
        .delete(DeleteOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "tenant-1".to_string(),
        })
        .expect("delete");
    assert!(deleted.deleted);

    // get -> None
    let gone = store
        .get(GetOp {
            namespace: ns,
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "tenant-1".to_string(),
        })
        .expect("get after delete");
    assert!(gone.is_none());
}

#[test]
fn pg_idempotency() {
    require_db!();
    let store = store();
    let ns = unique_namespace("idem");
    let op = || CreateOp {
        namespace: ns.clone(),
        entity: "Tenant".to_string(),
        collection: "tenants".to_string(),
        input: json!({"name": "Idem"}),
        idempotency_key: Some("req-1".to_string()),
        unique_indexes: Vec::new(),
        unique_behavior: UniqueConflictBehavior::Reject,
    };
    let first = store.create(op()).expect("first create");
    let second = store.create(op()).expect("second create");
    assert_eq!(first, second, "idempotent create returns same record");

    // only one record exists in the collection
    let all = store
        .query(QueryOp {
            namespace: ns,
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            filter: json!({}),
            order_by: Vec::new(),
        })
        .expect("query");
    assert_eq!(all.records.len(), 1);
}

#[test]
fn pg_unique_index_conflict() {
    require_db!();
    let store = store();
    let ns = unique_namespace("uniq");
    let index = UniqueIndex {
        id: "tenants_email".to_string(),
        record: "Tenant".to_string(),
        collection: "tenants".to_string(),
        fields: vec!["email".to_string()],
    };
    store
        .create(CreateOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            input: json!({"id": "t1", "email": "a@example.com"}),
            idempotency_key: None,
            unique_indexes: vec![index.clone()],
            unique_behavior: UniqueConflictBehavior::Reject,
        })
        .expect("first create");

    // Reject behavior -> error
    let err = store
        .create(CreateOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            input: json!({"id": "t2", "email": "a@example.com"}),
            idempotency_key: None,
            unique_indexes: vec![index.clone()],
            unique_behavior: UniqueConflictBehavior::Reject,
        })
        .expect_err("unique violation");
    assert_eq!(err.code, "unique_constraint_violation");

    // ReturnExisting behavior -> returns the original record
    let existing = store
        .create(CreateOp {
            namespace: ns,
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            input: json!({"id": "t3", "email": "a@example.com"}),
            idempotency_key: None,
            unique_indexes: vec![index],
            unique_behavior: UniqueConflictBehavior::ReturnExisting {
                index: "tenants_email".to_string(),
                fields: vec!["email".to_string()],
            },
        })
        .expect("return existing");
    assert_eq!(existing.id, "t1");
}

#[test]
fn pg_events_ordered() {
    require_db!();
    let store = store();
    let ns = unique_namespace("events");
    let mut sequences = Vec::new();
    for n in 0..3 {
        let record = store
            .append_event(AppendEventOp {
                namespace: ns.clone(),
                stream: "lease".to_string(),
                event_type: "lease.signed".to_string(),
                capability: None,
                producer: None,
                subject_entity: "Lease".to_string(),
                subject_id: "lease-1".to_string(),
                data: json!({"n": n}),
                occurred_at: chrono::Utc::now(),
            })
            .expect("append event");
        sequences.push(record.sequence);
    }
    assert_eq!(sequences, vec![1, 2, 3], "sequences strictly increasing");
}

#[test]
fn pg_evidence_and_external_refs() {
    require_db!();
    let store = store();
    let ns = unique_namespace("evidence");
    let subject = ExternalRefsOp {
        namespace: ns.clone(),
        entity: "Tenant".to_string(),
        collection: "tenants".to_string(),
        id: "tenant-1".to_string(),
    };

    // external refs empty by default
    let refs = store
        .get_external_refs(subject.clone())
        .expect("external refs");
    assert!(refs.refs.is_empty());

    // store evidence twice -> both retrievable in order
    store
        .store_evidence(StoreEvidenceOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "tenant-1".to_string(),
            evidence: json!({"kind": "doc", "ref": "a"}),
        })
        .expect("store evidence 1");
    store
        .store_evidence(StoreEvidenceOp {
            namespace: ns,
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "tenant-1".to_string(),
            evidence: json!({"kind": "doc", "ref": "b"}),
        })
        .expect("store evidence 2");

    let evidence = store.get_evidence(subject).expect("get evidence");
    assert_eq!(evidence.evidence.len(), 2);
    assert_eq!(evidence.evidence[0].get("ref"), Some(&json!("a")));
    assert_eq!(evidence.evidence[1].get("ref"), Some(&json!("b")));
}

#[test]
fn pg_restart_durability() {
    require_db!();
    let ns = unique_namespace("durability");
    {
        let store = store();
        store
            .create(CreateOp {
                namespace: ns.clone(),
                entity: "Tenant".to_string(),
                collection: "tenants".to_string(),
                input: json!({"id": "durable-1", "name": "Persist"}),
                idempotency_key: None,
                unique_indexes: Vec::new(),
                unique_behavior: UniqueConflictBehavior::Reject,
            })
            .expect("create");
    } // drop the first store

    let restarted = store();
    let fetched = restarted
        .get(GetOp {
            namespace: ns,
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "durable-1".to_string(),
        })
        .expect("get after reconnect")
        .expect("record persisted across store instances");
    assert_eq!(fetched.data.get("name"), Some(&json!("Persist")));
}

#[test]
fn pg_namespace_isolation() {
    require_db!();
    let store = store();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let ns_a = ProviderNamespace {
        tenant_id: format!("iso-a-{nanos}"),
        sor_name: "landlord".to_string(),
    };
    let ns_b = ProviderNamespace {
        tenant_id: format!("iso-b-{nanos}"),
        sor_name: "landlord".to_string(),
    };
    store
        .create(CreateOp {
            namespace: ns_a.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            input: json!({"id": "shared", "owner": "a"}),
            idempotency_key: None,
            unique_indexes: Vec::new(),
            unique_behavior: UniqueConflictBehavior::Reject,
        })
        .expect("create a");
    store
        .create(CreateOp {
            namespace: ns_b.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            input: json!({"id": "shared", "owner": "b"}),
            idempotency_key: None,
            unique_indexes: Vec::new(),
            unique_behavior: UniqueConflictBehavior::Reject,
        })
        .expect("create b");

    let a = store
        .get(GetOp {
            namespace: ns_a,
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "shared".to_string(),
        })
        .expect("get a")
        .expect("present a");
    let b = store
        .get(GetOp {
            namespace: ns_b,
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "shared".to_string(),
        })
        .expect("get b")
        .expect("present b");
    assert_eq!(a.data.get("owner"), Some(&json!("a")));
    assert_eq!(b.data.get("owner"), Some(&json!("b")));
}

fn email_index() -> UniqueIndex {
    UniqueIndex {
        id: "tenants_email".to_string(),
        record: "Tenant".to_string(),
        collection: "tenants".to_string(),
        fields: vec!["email".to_string()],
    }
}

fn create_with_email(ns: &ProviderNamespace, id: &str, email: &str) -> CreateOp {
    CreateOp {
        namespace: ns.clone(),
        entity: "Tenant".to_string(),
        collection: "tenants".to_string(),
        input: json!({"id": id, "email": email}),
        idempotency_key: None,
        unique_indexes: vec![email_index()],
        unique_behavior: UniqueConflictBehavior::Reject,
    }
}

/// Several writers race one unique value. SERIALIZABLE + retry must let
/// exactly one win; the rest must see the conflict, never a second row.
///
/// Twenty rounds of sixteen writers, with a pool as large as the writer
/// count, so the transactions genuinely overlap: with one round and a small
/// pool the writers serialize on connection checkout and the test passes even
/// at READ COMMITTED, proving nothing.
#[test]
fn pg_concurrent_writers_cannot_break_a_unique_index() {
    require_db!();
    let writers = 16;
    let store = Arc::new(
        PostgresStore::connect_url(&url().expect("url"), writers as u32).expect("connect"),
    );
    let ns = unique_namespace("race");
    for round in 0..20 {
        let email = format!("same-{round}@example.com");
        let barrier = Arc::new(Barrier::new(writers));
        let handles: Vec<_> = (0..writers)
            .map(|i| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                let ns = ns.clone();
                let email = email.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.create(create_with_email(&ns, &format!("r{round}-t{i}"), &email))
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().expect("join"))
            .collect();
        let won = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(
            won, 1,
            "round {round}: exactly one writer may take the value: {results:?}"
        );
        for err in results.iter().filter_map(|r| r.as_ref().err()) {
            assert_eq!(
                err.code, "unique_constraint_violation",
                "round {round}: {err:?}"
            );
        }
    }
    let rows = store
        .query(QueryOp {
            namespace: ns,
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            filter: json!({}),
            order_by: Vec::new(),
        })
        .expect("query");
    assert_eq!(rows.records.len(), 20, "one record per round");
}

/// Changing a unique field frees the old value for another record.
#[test]
fn pg_an_updated_unique_value_is_released() {
    require_db!();
    let store = store();
    let ns = unique_namespace("release");
    store
        .create(create_with_email(&ns, "t1", "old@example.com"))
        .expect("t1");
    store
        .update(UpdateOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "t1".to_string(),
            patch: json!({"email": "new@example.com"}),
            unique_indexes: vec![email_index()],
        })
        .expect("update");
    store
        .create(create_with_email(&ns, "t2", "old@example.com"))
        .expect("the released value is free again");
    let err = store
        .create(create_with_email(&ns, "t3", "new@example.com"))
        .expect_err("the new value is held");
    assert_eq!(err.code, "unique_constraint_violation");
}

#[test]
fn pg_is_its_own_durable_migration_ledger() {
    require_db!();
    let ns = unique_namespace("ledger");
    {
        let store = store();
        let ledger = store.as_migration_ledger().expect("postgres is a ledger");
        ledger.record_applied(&ns, "0001_init").expect("record");
        ledger.record_applied(&ns, "0001_init").expect("idempotent");
        ledger.record_applied(&ns, "0002_more").expect("record 2");
    }
    let store = store();
    let applied = store
        .as_migration_ledger()
        .expect("ledger")
        .load(&ns)
        .expect("load");
    assert!(applied.contains("0001_init"));
    assert!(applied.contains("0002_more"));
    let other = store
        .as_migration_ledger()
        .expect("ledger")
        .load(&unique_namespace("ledger-other"))
        .expect("load other");
    assert!(!other.contains("0001_init"), "the ledger is per namespace");
}

fn create_auto(ns: &ProviderNamespace, label: &str) -> CreateOp {
    CreateOp {
        namespace: ns.clone(),
        entity: "Order".to_string(),
        collection: "orders".to_string(),
        input: json!({"label": label}),
        idempotency_key: None,
        unique_indexes: Vec::new(),
        unique_behavior: UniqueConflictBehavior::Reject,
    }
}

/// An update that names NO indexes (a migration backfill, a manager submit)
/// must leave every unique index's protection in place.
#[test]
fn pg_an_update_naming_no_indexes_keeps_unique_protection() {
    require_db!();
    let store = store();
    let ns = unique_namespace("keep-uniq");
    store
        .create(create_with_email(&ns, "t1", "a@example.com"))
        .expect("t1");
    store
        .update(UpdateOp {
            namespace: ns.clone(),
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "t1".to_string(),
            patch: json!({"plan": "basic"}),
            unique_indexes: Vec::new(),
        })
        .expect("backfill-style update");
    let err = store
        .create(create_with_email(&ns, "t2", "a@example.com"))
        .expect_err("the value is still held by t1");
    assert_eq!(err.code, "unique_constraint_violation");
}

/// Deleting an earlier record must not make the next auto-id land on a later
/// record that still exists (a count-derived id would, and overwrite it).
#[test]
fn pg_an_auto_id_never_reuses_a_live_record_after_a_delete() {
    require_db!();
    let store = store();
    let ns = unique_namespace("autoid");
    let first = store.create(create_auto(&ns, "first")).expect("first");
    let second = store.create(create_auto(&ns, "second")).expect("second");
    store
        .delete(DeleteOp {
            namespace: ns.clone(),
            entity: "Order".to_string(),
            collection: "orders".to_string(),
            id: first.id.clone(),
        })
        .expect("delete first");
    let third = store.create(create_auto(&ns, "third")).expect("third");
    assert_ne!(third.id, second.id, "the new record took a live id");
    let still = store
        .get(GetOp {
            namespace: ns,
            entity: "Order".to_string(),
            collection: "orders".to_string(),
            id: second.id,
        })
        .expect("get")
        .expect("second still exists");
    assert_eq!(still.data["label"], "second", "second was overwritten");
}

/// Concurrent creates without ids all succeed with distinct ids: they contend
/// on the collection counter and retry within the time budget.
#[test]
fn pg_concurrent_auto_id_creates_all_succeed_with_distinct_ids() {
    require_db!();
    let writers = 16;
    let store = Arc::new(
        PostgresStore::connect_url(&url().expect("url"), writers as u32).expect("connect"),
    );
    let ns = unique_namespace("autoid-race");
    let barrier = Arc::new(Barrier::new(writers));
    let handles: Vec<_> = (0..writers)
        .map(|i| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let ns = ns.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.create(create_auto(&ns, &format!("w{i}")))
            })
        })
        .collect();
    let mut ids: Vec<String> = handles
        .into_iter()
        .map(|h| h.join().expect("join").expect("every create succeeds").id)
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), writers, "ids must be distinct: {ids:?}");
}

/// A role granted only DML on a table a DBA created must be able to boot.
#[test]
fn pg_a_dml_only_role_connects_to_an_existing_table() {
    require_db!();
    let admin = url().expect("url");
    // Make sure the table exists (created by the admin connection).
    drop(store());
    let role = format!("sorx_dml_{}", std::process::id());
    let mut client = postgres::Client::connect(&admin, postgres::NoTls).expect("admin");
    client
        .batch_execute(&format!(
            "DROP ROLE IF EXISTS {role}; \
             CREATE ROLE {role} LOGIN PASSWORD 'dml'; \
             REVOKE CREATE ON SCHEMA public FROM PUBLIC; \
             GRANT USAGE ON SCHEMA public TO {role}; \
             GRANT SELECT, INSERT, UPDATE, DELETE ON sorx_kv TO {role};"
        ))
        .expect("create role");
    let mut parsed: postgres::Config = admin.parse().expect("parse");
    parsed.user(&role).password("dml");
    let host = match &parsed.get_hosts()[0] {
        postgres::config::Host::Tcp(host) => host.clone(),
        other => panic!("tcp host expected: {other:?}"),
    };
    let port = parsed.get_ports().first().copied().unwrap_or(5432);
    let db = parsed.get_dbname().unwrap_or("postgres").to_string();
    let dml_url = format!("postgres://{role}:dml@{host}:{port}/{db}?sslmode=disable");
    let result = PostgresStore::connect_url(&dml_url, 1);
    client
        .batch_execute(&format!(
            "REVOKE ALL ON sorx_kv FROM {role}; REVOKE ALL ON SCHEMA public FROM {role}; \
             DROP ROLE {role};"
        ))
        .expect("cleanup");
    result.expect("a DML-only role must boot against an existing table");
}
