use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    AppendEventOp, CreateOp, DeleteOp, DeleteResult, EntityRecord, EventRecord, EvidenceResult,
    ExternalRefsOp, ExternalRefsResult, GetOp, IndexQueryOp, IndexQueryResult, QueryOp,
    QueryResult, SorStoreProvider, SorxCanonicalStore, SorxResult, StoreEvidenceOp, TraverseOp,
    TraverseResult, UpdateOp,
};

use super::MemoryStoreProvider;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundationDbProviderConfig {
    pub cluster_file: Option<String>,
    pub database: Option<String>,
    pub config_ref: Option<String>,
}

impl FoundationDbProviderConfig {
    pub fn from_parts(config_ref: Option<String>, config: Option<Value>) -> Self {
        let object = config.and_then(|value| value.as_object().cloned());
        Self {
            cluster_file: object
                .as_ref()
                .and_then(|value| string_field(value, "cluster_file")),
            database: object
                .as_ref()
                .and_then(|value| string_field(value, "database")),
            config_ref,
        }
    }
}

#[derive(Debug)]
pub struct FoundationDbProviderAdapter {
    inner: MemoryStoreProvider,
}

impl FoundationDbProviderAdapter {
    pub fn new(config: FoundationDbProviderConfig) -> SorxResult<Self> {
        Ok(Self {
            inner: MemoryStoreProvider::persistent(config.persistence_path())?,
        })
    }

    pub fn unavailable(config: FoundationDbProviderConfig) -> Self {
        Self::new(config).expect("local FoundationDB adapter persistence path must initialize")
    }
}

impl SorStoreProvider for FoundationDbProviderAdapter {
    fn create(&self, op: CreateOp) -> SorxResult<EntityRecord> {
        self.inner.create(op)
    }

    fn get(&self, op: GetOp) -> SorxResult<Option<EntityRecord>> {
        self.inner.get(op)
    }

    fn update(&self, op: UpdateOp) -> SorxResult<EntityRecord> {
        self.inner.update(op)
    }

    fn query(&self, op: QueryOp) -> SorxResult<QueryResult> {
        self.inner.query(op)
    }

    fn delete(&self, op: DeleteOp) -> SorxResult<DeleteResult> {
        self.inner.delete(op)
    }
}

impl SorxCanonicalStore for FoundationDbProviderAdapter {
    fn append_event(&self, op: AppendEventOp) -> SorxResult<EventRecord> {
        self.inner.append_event(op)
    }

    fn query_index(&self, op: IndexQueryOp) -> SorxResult<IndexQueryResult> {
        self.inner.query_index(op)
    }

    fn traverse(&self, op: TraverseOp) -> SorxResult<TraverseResult> {
        self.inner.traverse(op)
    }

    fn get_external_refs(&self, op: ExternalRefsOp) -> SorxResult<ExternalRefsResult> {
        self.inner.get_external_refs(op)
    }

    fn store_evidence(&self, op: StoreEvidenceOp) -> SorxResult<()> {
        self.inner.store_evidence(op)
    }

    fn get_evidence(&self, op: ExternalRefsOp) -> SorxResult<EvidenceResult> {
        self.inner.get_evidence(op)
    }
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(ToString::to_string)
}

impl FoundationDbProviderConfig {
    fn persistence_path(&self) -> PathBuf {
        if let Some(database) = self.database.as_deref()
            && (database.contains('/') || database.ends_with(".json"))
        {
            return PathBuf::from(database);
        }
        let mut hasher = Sha256::new();
        hasher.update(self.config_ref.as_deref().unwrap_or("direct"));
        hasher.update(b"\0");
        hasher.update(self.cluster_file.as_deref().unwrap_or(""));
        hasher.update(b"\0");
        hasher.update(self.database.as_deref().unwrap_or("default"));
        let digest = hex_prefix(&hasher.finalize());
        std::env::temp_dir().join(format!("greentic-sorx-foundationdb-{digest}.json"))
    }
}

fn hex_prefix(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
