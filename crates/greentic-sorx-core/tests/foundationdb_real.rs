//! Integration tests for the real FoundationDB-backed canonical store.
//!
//! These tests are compiled only with `--features foundationdb` and require a
//! live FoundationDB 7.3 cluster reachable via `FDB_CLUSTER_FILE`. They MUST be
//! run through the FDB build harness (`/tmp/fdb-build.sh`) so the client library
//! links and the cluster file is set.
//!
//! Each test uses a unique tenant slug so reruns never collide.
#![cfg(feature = "foundationdb")]

use std::time::{SystemTime, UNIX_EPOCH};

use greentic_sorx_core::providers::foundationdb_real::FoundationDbStore;
use greentic_sorx_core::{
    AppendEventOp, CreateOp, DeleteOp, ExternalRefsOp, GetOp, ProviderNamespace, QueryOp,
    QueryOrder, QueryOrderDirection, SorStoreProvider, SorxCanonicalStore, StoreEvidenceOp,
    UniqueConflictBehavior, UniqueIndex, UpdateOp,
};
use serde_json::json;

/// Cluster file the harness exports. Falls back to the conventional dev path.
fn cluster_file() -> String {
    std::env::var("FDB_CLUSTER_FILE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
            format!("{home}/.local/fdb/fdb.cluster")
        })
}

/// Unique tenant slug per test invocation (nanos + test name) so concurrent and
/// repeated runs never share a keyspace.
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

fn store() -> FoundationDbStore {
    FoundationDbStore::connect(&cluster_file()).expect("connect to live FoundationDB cluster")
}

#[test]
fn fdb_crud_roundtrip() {
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
fn fdb_idempotency() {
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
fn fdb_unique_index_conflict() {
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
fn fdb_events_ordered() {
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
fn fdb_evidence_and_external_refs() {
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
fn fdb_restart_durability() {
    let cluster = cluster_file();
    let ns = unique_namespace("durability");
    {
        let store = FoundationDbStore::connect(&cluster).expect("connect 1");
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

    let restarted = FoundationDbStore::connect(&cluster).expect("connect 2");
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
fn fdb_namespace_isolation() {
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
