use std::sync::{Arc, Mutex};

use greentic_sorx_core::{
    BindingResolver, CreateOp, DeleteOp, DeleteResult, EndpointRouter, EntityRecord,
    FoundationDbProviderAdapter, FoundationDbProviderConfig, GetOp, MemoryStoreProvider,
    ProviderBinding, ProviderNamespace, ProviderRegistry, QueryOp, QueryResult, SorStoreProvider,
    SorxRuntime, StoreProviderKind, UpdateOp, default_start_schema, invocation,
    runtime_config_from_answers, runtime_pack,
};
use serde_json::{Value, json};

fn gateway() -> Value {
    json!({
        "schema": "greentic.sorla.agent-gateway.v1",
        "endpoints": [
            {
                "endpoint_id": "tenant.create",
                "operation_id": "tenant.create",
                "operation": "create",
                "method": "POST",
                "path": "/v1/tenants",
                "entity": "Tenant",
                "collection": "gateway_tenants",
                "provider_binding": "store",
                "risk": "low"
            },
            {
                "endpoint_id": "property.create",
                "operation_id": "property.create",
                "operation": "create",
                "method": "POST",
                "path": "/v1/properties",
                "entity": "Property",
                "collection": "properties",
                "provider_binding": "store",
                "risk": "low"
            }
        ]
    })
}

fn answers(bindings: Value) -> Value {
    json!({
        "tenant": { "tenant_id": "tenant-a", "environment": "local" },
        "server": { "bind": "127.0.0.1:0", "public_base_url": "http://127.0.0.1:0" },
        "providers": { "store": { "kind": "memory", "config_ref": "providers.memory.local" } },
        "bindings": bindings,
        "policy": { "approvals": {} },
        "audit": {},
        "deployment": {
            "tenant_id": "tenant-a",
            "sor_name": "landlord",
            "environment": "local"
        },
        "exposure": {},
        "ghcr": {}
    })
}

fn runtime_with_provider(provider: Arc<dyn SorStoreProvider>, bindings: Value) -> SorxRuntime {
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let config = runtime_config_from_answers("landlord", &answers(bindings)).unwrap();
    let mut providers = ProviderRegistry::new();
    providers.register_store("store", provider);
    SorxRuntime::new(runtime_pack("landlord", "0.1.0"), config, router, providers)
}

#[derive(Debug, Default)]
struct CapturingProvider {
    creates: Mutex<Vec<CreateOp>>,
}

impl CapturingProvider {
    fn first_create(&self) -> CreateOp {
        self.creates.lock().unwrap()[0].clone()
    }
}

impl SorStoreProvider for CapturingProvider {
    fn create(&self, op: CreateOp) -> greentic_sorx_core::SorxResult<EntityRecord> {
        self.creates.lock().unwrap().push(op.clone());
        Ok(EntityRecord {
            entity: op.entity,
            collection: op.collection,
            id: "created-1".to_string(),
            data: json!({ "id": "created-1" }),
            version: 1,
        })
    }

    fn get(&self, _op: GetOp) -> greentic_sorx_core::SorxResult<Option<EntityRecord>> {
        Ok(None)
    }

    fn update(&self, _op: UpdateOp) -> greentic_sorx_core::SorxResult<EntityRecord> {
        unreachable!("update is not used in this test")
    }

    fn query(&self, _op: QueryOp) -> greentic_sorx_core::SorxResult<QueryResult> {
        Ok(QueryResult { records: vec![] })
    }

    fn delete(&self, _op: DeleteOp) -> greentic_sorx_core::SorxResult<DeleteResult> {
        Ok(DeleteResult { deleted: false })
    }
}

#[test]
fn entity_binding_resolves_provider_and_collection() {
    let mut bindings = std::collections::HashMap::new();
    bindings.insert(
        "Tenant".to_string(),
        ProviderBinding {
            entity: "Tenant".to_string(),
            provider_id: "store".to_string(),
            collection: "tenant_records".to_string(),
        },
    );
    let resolver = BindingResolver::new(bindings);
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let resolved = resolver
        .resolve(router.endpoint("tenant.create").unwrap())
        .unwrap();
    assert_eq!(resolved.provider_id, "store");
    assert_eq!(resolved.collection, "tenant_records");
}

#[test]
fn default_binding_uses_gateway_provider_and_collection_when_bindings_are_omitted() {
    let resolver = BindingResolver::new(std::collections::HashMap::new());
    let router = EndpointRouter::from_agent_gateway(&gateway()).unwrap();
    let resolved = resolver
        .resolve(router.endpoint("tenant.create").unwrap())
        .unwrap();
    assert_eq!(resolved.provider_id, "store");
    assert_eq!(resolved.collection, "gateway_tenants");
}

#[test]
fn missing_explicit_binding_fails_clearly() {
    let runtime = runtime_with_provider(
        Arc::new(MemoryStoreProvider::new()),
        json!({ "entities": { "Property": { "provider": "store", "collection": "properties" } } }),
    );
    let err = runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "name": "Acme" }),
        ))
        .unwrap_err();
    assert_eq!(err.code, "provider_binding_missing");
    assert!(err.message.contains("Tenant"));
}

#[test]
fn runtime_applies_binding_collection_and_tenant_namespace_to_provider_ops() {
    let provider = Arc::new(CapturingProvider::default());
    let runtime = runtime_with_provider(
        provider.clone(),
        json!({ "entities": { "Tenant": { "provider": "store", "collection": "tenant_records" } } }),
    );
    runtime
        .invoke(invocation(
            "tenant-a",
            "tenant.create",
            "tenant.create",
            json!({ "name": "Acme" }),
        ))
        .unwrap();

    let op = provider.first_create();
    assert_eq!(op.entity, "Tenant");
    assert_eq!(op.collection, "tenant_records");
    assert_eq!(
        op.namespace,
        ProviderNamespace {
            tenant_id: "tenant-a".to_string(),
            pack_name: "landlord".to_string(),
            pack_version: "0.1.0".to_string()
        }
    );
}

#[test]
fn memory_provider_namespaces_records_by_tenant_and_pack() {
    let provider = MemoryStoreProvider::new();
    let mut create = CreateOp {
        namespace: ProviderNamespace {
            tenant_id: "tenant-a".to_string(),
            pack_name: "landlord".to_string(),
            pack_version: "0.1.0".to_string(),
        },
        entity: "Tenant".to_string(),
        collection: "tenants".to_string(),
        input: json!({ "id": "same-id", "name": "A" }),
        idempotency_key: None,
    };
    provider.create(create.clone()).unwrap();
    create.namespace.tenant_id = "tenant-b".to_string();
    create.input = json!({ "id": "same-id", "name": "B" });
    provider.create(create).unwrap();

    let tenant_a = provider
        .get(GetOp {
            namespace: ProviderNamespace {
                tenant_id: "tenant-a".to_string(),
                pack_name: "landlord".to_string(),
                pack_version: "0.1.0".to_string(),
            },
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "same-id".to_string(),
        })
        .unwrap()
        .unwrap();
    let tenant_b = provider
        .get(GetOp {
            namespace: ProviderNamespace {
                tenant_id: "tenant-b".to_string(),
                pack_name: "landlord".to_string(),
                pack_version: "0.1.0".to_string(),
            },
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "same-id".to_string(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(tenant_a.data["name"], "A");
    assert_eq!(tenant_b.data["name"], "B");
}

#[test]
fn provider_kind_parses_memory_foundationdb_and_external() {
    assert_eq!(
        StoreProviderKind::parse("memory"),
        StoreProviderKind::Memory
    );
    assert_eq!(
        StoreProviderKind::parse("foundationdb"),
        StoreProviderKind::FoundationDb
    );
    assert_eq!(
        StoreProviderKind::parse("postgres"),
        StoreProviderKind::External("postgres".to_string())
    );
}

#[test]
fn foundationdb_adapter_fails_with_clear_unavailable_error() {
    let provider = FoundationDbProviderAdapter::unavailable(FoundationDbProviderConfig {
        cluster_file: None,
        database: None,
        config_ref: Some("providers.foundationdb.local".to_string()),
    });
    let err = provider
        .get(GetOp {
            namespace: ProviderNamespace {
                tenant_id: "tenant-a".to_string(),
                pack_name: "landlord".to_string(),
                pack_version: "0.1.0".to_string(),
            },
            entity: "Tenant".to_string(),
            collection: "tenants".to_string(),
            id: "tenant-1".to_string(),
        })
        .unwrap_err();
    assert_eq!(err.code, "provider_unavailable");
    assert!(err.message.contains("SORX store provider"));
}

#[test]
fn foundationdb_config_parses_direct_fields_and_config_ref() {
    let config = FoundationDbProviderConfig::from_parts(
        Some("providers.foundationdb.prod".to_string()),
        Some(json!({
            "cluster_file": "/etc/foundationdb/fdb.cluster",
            "database": "tenant-db",
            "ignored": 42
        })),
    );
    assert_eq!(
        config.cluster_file.as_deref(),
        Some("/etc/foundationdb/fdb.cluster")
    );
    assert_eq!(config.database.as_deref(), Some("tenant-db"));
    assert_eq!(
        config.config_ref.as_deref(),
        Some("providers.foundationdb.prod")
    );

    let empty = FoundationDbProviderConfig::from_parts(None, Some(json!("not-an-object")));
    assert_eq!(empty.cluster_file, None);
    assert_eq!(empty.database, None);
    assert_eq!(empty.config_ref, None);
}

#[test]
fn foundationdb_adapter_rejects_all_store_operations_until_wired() {
    let provider = FoundationDbProviderAdapter::unavailable(FoundationDbProviderConfig {
        cluster_file: Some("./fdb.cluster".to_string()),
        database: Some("DB".to_string()),
        config_ref: None,
    });
    let namespace = ProviderNamespace {
        tenant_id: "tenant-a".to_string(),
        pack_name: "landlord".to_string(),
        pack_version: "0.1.0".to_string(),
    };
    let create = provider.create(CreateOp {
        namespace: namespace.clone(),
        entity: "Tenant".to_string(),
        collection: "tenants".to_string(),
        input: json!({"id": "tenant-1"}),
        idempotency_key: None,
    });
    let update = provider.update(UpdateOp {
        namespace: namespace.clone(),
        entity: "Tenant".to_string(),
        collection: "tenants".to_string(),
        id: "tenant-1".to_string(),
        patch: json!({"active": true}),
    });
    let query = provider.query(QueryOp {
        namespace: namespace.clone(),
        entity: "Tenant".to_string(),
        collection: "tenants".to_string(),
        filter: json!({"active": true}),
    });
    let delete = provider.delete(DeleteOp {
        namespace,
        entity: "Tenant".to_string(),
        collection: "tenants".to_string(),
        id: "tenant-1".to_string(),
    });

    for err in [
        create.unwrap_err(),
        update.unwrap_err(),
        query.unwrap_err(),
        delete.unwrap_err(),
    ] {
        assert_eq!(err.code, "provider_unavailable");
        assert!(err.message.contains("direct local/test config"));
    }
}

#[test]
fn config_ref_is_accepted_without_exposing_secret_values() {
    let normalized = greentic_sorx_core::normalize_start_answers(
        &default_start_schema(),
        &answers(json!({})),
        true,
    )
    .unwrap();
    assert_eq!(
        normalized.answers["providers"]["store"]["config_ref"],
        "providers.memory.local"
    );
}

#[test]
fn direct_provider_config_is_only_allowed_in_local_or_test_mode() {
    let mut local_answers = answers(json!({}));
    local_answers["providers"]["store"] = json!({
        "kind": "foundationdb",
        "config": {
            "cluster_file": "./.local/fdb.cluster",
            "database": "DB"
        }
    });
    greentic_sorx_core::normalize_start_answers(&default_start_schema(), &local_answers, true)
        .unwrap();

    let mut dev_answers = local_answers;
    dev_answers["tenant"]["environment"] = json!("dev");
    let err =
        greentic_sorx_core::normalize_start_answers(&default_start_schema(), &dev_answers, true)
            .unwrap_err();
    assert_eq!(err.code, "invalid_answers");
    assert!(
        err.issues
            .iter()
            .any(|issue| issue.path == "providers.store.config")
    );
}
