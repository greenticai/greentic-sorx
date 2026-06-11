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

/// Backing store for the FoundationDB binding.
///
/// The adapter chooses its backend at construction time:
///
/// * With `--features foundationdb` AND a configured `cluster_file` that exists
///   on disk, it wires the real [`FoundationDbStore`](super::FoundationDbStore)
///   against the live cluster.
/// * Otherwise (feature off, or no/absent cluster file) it falls back to the
///   file-persistent [`MemoryStoreProvider`] — the historical stub behavior.
enum Backend {
    Memory(MemoryStoreProvider),
    #[cfg(feature = "foundationdb")]
    Foundation(super::FoundationDbStore),
}

#[derive(Debug)]
pub struct FoundationDbProviderAdapter {
    inner: Backend,
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(_) => f.write_str("Backend::Memory"),
            #[cfg(feature = "foundationdb")]
            Self::Foundation(_) => f.write_str("Backend::Foundation"),
        }
    }
}

impl FoundationDbProviderAdapter {
    pub fn new(config: FoundationDbProviderConfig) -> SorxResult<Self> {
        #[cfg(feature = "foundationdb")]
        {
            // Only engage the real backend when a cluster file is configured
            // and actually present; this keeps the existing stub tests (which
            // pass placeholder cluster paths) on the memory backend.
            if let Some(cluster_file) = config.cluster_file.as_deref()
                && std::path::Path::new(cluster_file).exists()
            {
                let store = super::FoundationDbStore::connect(cluster_file)?;
                return Ok(Self {
                    inner: Backend::Foundation(store),
                });
            }
        }
        Ok(Self {
            inner: Backend::Memory(MemoryStoreProvider::persistent(config.persistence_path())?),
        })
    }

    pub fn unavailable(config: FoundationDbProviderConfig) -> Self {
        Self::new(config).expect("local FoundationDB adapter persistence path must initialize")
    }
}

impl SorStoreProvider for FoundationDbProviderAdapter {
    fn create(&self, op: CreateOp) -> SorxResult<EntityRecord> {
        match &self.inner {
            Backend::Memory(inner) => inner.create(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.create(op),
        }
    }

    fn get(&self, op: GetOp) -> SorxResult<Option<EntityRecord>> {
        match &self.inner {
            Backend::Memory(inner) => inner.get(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.get(op),
        }
    }

    fn update(&self, op: UpdateOp) -> SorxResult<EntityRecord> {
        match &self.inner {
            Backend::Memory(inner) => inner.update(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.update(op),
        }
    }

    fn query(&self, op: QueryOp) -> SorxResult<QueryResult> {
        match &self.inner {
            Backend::Memory(inner) => inner.query(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.query(op),
        }
    }

    fn delete(&self, op: DeleteOp) -> SorxResult<DeleteResult> {
        match &self.inner {
            Backend::Memory(inner) => inner.delete(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.delete(op),
        }
    }
}

impl SorxCanonicalStore for FoundationDbProviderAdapter {
    fn append_event(&self, op: AppendEventOp) -> SorxResult<EventRecord> {
        match &self.inner {
            Backend::Memory(inner) => inner.append_event(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.append_event(op),
        }
    }

    fn query_index(&self, op: IndexQueryOp) -> SorxResult<IndexQueryResult> {
        match &self.inner {
            Backend::Memory(inner) => inner.query_index(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.query_index(op),
        }
    }

    fn traverse(&self, op: TraverseOp) -> SorxResult<TraverseResult> {
        match &self.inner {
            Backend::Memory(inner) => inner.traverse(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.traverse(op),
        }
    }

    fn get_external_refs(&self, op: ExternalRefsOp) -> SorxResult<ExternalRefsResult> {
        match &self.inner {
            Backend::Memory(inner) => inner.get_external_refs(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.get_external_refs(op),
        }
    }

    fn store_evidence(&self, op: StoreEvidenceOp) -> SorxResult<()> {
        match &self.inner {
            Backend::Memory(inner) => inner.store_evidence(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.store_evidence(op),
        }
    }

    fn get_evidence(&self, op: ExternalRefsOp) -> SorxResult<EvidenceResult> {
        match &self.inner {
            Backend::Memory(inner) => inner.get_evidence(op),
            #[cfg(feature = "foundationdb")]
            Backend::Foundation(inner) => inner.get_evidence(op),
        }
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
