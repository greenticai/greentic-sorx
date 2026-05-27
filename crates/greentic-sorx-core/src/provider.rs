use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{EndpointDefinition, SorxError, SorxResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreProviderKind {
    Memory,
    FoundationDb,
    External(String),
}

impl StoreProviderKind {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "memory" => Self::Memory,
            "foundationdb" | "foundation_db" | "foundation-db" => Self::FoundationDb,
            other => Self::External(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Memory => "memory",
            Self::FoundationDb => "foundationdb",
            Self::External(value) => value.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBinding {
    pub entity: String,
    pub provider_id: String,
    pub collection: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderNamespace {
    pub tenant_id: String,
    pub sor_name: String,
}

impl ProviderNamespace {
    pub fn key_prefix(&self) -> String {
        format!(
            "sorx/{}/{}",
            clean_segment(&self.tenant_id),
            clean_segment(&self.sor_name)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingResolver {
    pub entity_bindings: HashMap<String, ProviderBinding>,
    default_provider_id: String,
}

impl BindingResolver {
    pub fn new(entity_bindings: HashMap<String, ProviderBinding>) -> Self {
        Self {
            entity_bindings,
            default_provider_id: "store".to_string(),
        }
    }

    pub fn with_default_provider_id(mut self, default_provider_id: impl Into<String>) -> Self {
        self.default_provider_id = default_provider_id.into();
        self
    }

    pub fn default_provider_id(&self) -> &str {
        &self.default_provider_id
    }

    pub fn resolve(&self, endpoint: &EndpointDefinition) -> SorxResult<ProviderBinding> {
        let entity = endpoint
            .entity
            .clone()
            .unwrap_or_else(|| endpoint.collection.clone());

        if let Some(binding) = self.entity_bindings.get(&entity) {
            return Ok(binding.clone());
        }

        if self.entity_bindings.is_empty() {
            return Ok(ProviderBinding {
                entity,
                provider_id: endpoint.provider_binding.clone(),
                collection: endpoint.collection.clone(),
            });
        }

        Err(SorxError::new(
            "provider_binding_missing",
            format!("missing provider binding for entity `{entity}`"),
        ))
    }

    pub fn from_config(
        bindings: &Map<String, Value>,
        default_provider_id: impl Into<String>,
    ) -> SorxResult<Self> {
        let default_provider_id = default_provider_id.into();
        let entities = bindings
            .get("entities")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut entity_bindings = HashMap::new();
        for (entity, value) in entities {
            let object = value.as_object().ok_or_else(|| {
                SorxError::at_path(
                    "provider_binding_invalid",
                    "entity binding must be an object",
                    format!("bindings.entities.{entity}"),
                )
            })?;
            let provider_id =
                string_field(object, "provider").unwrap_or(default_provider_id.clone());
            let collection = string_field(object, "collection")
                .unwrap_or_else(|| default_collection_name(&entity));
            entity_bindings.insert(
                entity.clone(),
                ProviderBinding {
                    entity,
                    provider_id,
                    collection,
                },
            );
        }
        Ok(Self::new(entity_bindings).with_default_provider_id(default_provider_id))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityRecord {
    pub entity: String,
    pub collection: String,
    pub id: String,
    pub data: Value,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateOp {
    pub namespace: ProviderNamespace,
    pub entity: String,
    pub collection: String,
    pub input: Value,
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub unique_indexes: Vec<UniqueIndex>,
    #[serde(default)]
    pub unique_behavior: UniqueConflictBehavior,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetOp {
    pub namespace: ProviderNamespace,
    pub entity: String,
    pub collection: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateOp {
    pub namespace: ProviderNamespace,
    pub entity: String,
    pub collection: String,
    pub id: String,
    pub patch: Value,
    #[serde(default)]
    pub unique_indexes: Vec<UniqueIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniqueIndex {
    pub id: String,
    pub record: String,
    pub collection: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniqueConflictBehavior {
    #[default]
    Reject,
    ReturnExisting {
        index: String,
        fields: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryOp {
    pub namespace: ProviderNamespace,
    pub entity: String,
    pub collection: String,
    pub filter: Value,
    #[serde(default)]
    pub order_by: Vec<QueryOrder>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryOrder {
    pub field: String,
    #[serde(default)]
    pub direction: QueryOrderDirection,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryOrderDirection {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteOp {
    pub namespace: ProviderNamespace,
    pub entity: String,
    pub collection: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub records: Vec<EntityRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResult {
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppendEventOp {
    pub namespace: ProviderNamespace,
    pub stream: String,
    pub event_type: String,
    pub subject_entity: String,
    pub subject_id: String,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub event_id: String,
    pub stream: String,
    pub event_type: String,
    pub subject_entity: String,
    pub subject_id: String,
    pub data: Value,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexQueryOp {
    pub namespace: ProviderNamespace,
    pub entity: String,
    pub collection: String,
    pub index: String,
    pub filter: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexQueryResult {
    pub records: Vec<EntityRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraverseOp {
    pub namespace: ProviderNamespace,
    pub root_entity: String,
    pub root_collection: String,
    pub root_id: String,
    pub max_depth: u8,
    pub relationships: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraverseResult {
    pub records: Vec<EntityRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRef {
    pub entity: String,
    pub id: String,
    pub system: String,
    pub external_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalRefsOp {
    pub namespace: ProviderNamespace,
    pub entity: String,
    pub collection: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalRefsResult {
    pub refs: Vec<ExternalRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreEvidenceOp {
    pub namespace: ProviderNamespace,
    pub entity: String,
    pub collection: String,
    pub id: String,
    pub evidence: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceResult {
    pub evidence: Vec<Value>,
}

pub trait SorStoreProvider: Send + Sync {
    fn create(&self, op: CreateOp) -> SorxResult<EntityRecord>;
    fn get(&self, op: GetOp) -> SorxResult<Option<EntityRecord>>;
    fn update(&self, op: UpdateOp) -> SorxResult<EntityRecord>;
    fn query(&self, op: QueryOp) -> SorxResult<QueryResult>;
    fn delete(&self, op: DeleteOp) -> SorxResult<DeleteResult>;
}

pub trait SorxCanonicalStore: SorStoreProvider {
    fn append_event(&self, op: AppendEventOp) -> SorxResult<EventRecord>;
    fn query_index(&self, op: IndexQueryOp) -> SorxResult<IndexQueryResult>;
    fn traverse(&self, op: TraverseOp) -> SorxResult<TraverseResult>;
    fn get_external_refs(&self, op: ExternalRefsOp) -> SorxResult<ExternalRefsResult>;
    fn store_evidence(&self, op: StoreEvidenceOp) -> SorxResult<()>;
    fn get_evidence(&self, op: ExternalRefsOp) -> SorxResult<EvidenceResult>;
}

#[derive(Clone, Default)]
pub struct ProviderRegistry {
    stores: BTreeMap<String, Arc<dyn SorStoreProvider>>,
    canonical_stores: BTreeMap<String, Arc<dyn SorxCanonicalStore>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_store(
        &mut self,
        binding: impl Into<String>,
        provider: Arc<dyn SorStoreProvider>,
    ) {
        self.stores.insert(binding.into(), provider);
    }

    pub fn register_canonical_store<P>(&mut self, binding: impl Into<String>, provider: Arc<P>)
    where
        P: SorxCanonicalStore + 'static,
    {
        let binding = binding.into();
        self.stores.insert(binding.clone(), provider.clone());
        self.canonical_stores.insert(binding, provider);
    }

    pub fn store(&self, binding: &str) -> SorxResult<Arc<dyn SorStoreProvider>> {
        self.stores.get(binding).cloned().ok_or_else(|| {
            SorxError::new(
                "provider_missing",
                format!("missing store provider binding `{binding}`"),
            )
        })
    }

    pub fn canonical_store(&self, binding: &str) -> SorxResult<Arc<dyn SorxCanonicalStore>> {
        self.canonical_stores.get(binding).cloned().ok_or_else(|| {
            SorxError::new(
                "provider_capability_missing",
                format!("store provider binding `{binding}` does not support canonical operations"),
            )
        })
    }

    pub fn contains_store(&self, binding: &str) -> bool {
        self.stores.contains_key(binding)
    }
}

pub fn default_collection_name(entity: &str) -> String {
    let snake = entity
        .chars()
        .enumerate()
        .fold(String::new(), |mut out, (index, ch)| {
            if ch.is_ascii_uppercase() {
                if index > 0 {
                    out.push('_');
                }
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push(ch);
            }
            out
        });
    format!("{snake}s")
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(ToString::to_string)
}

fn clean_segment(value: &str) -> String {
    value
        .trim_matches('/')
        .replace(['/', '\\'], "_")
        .chars()
        .filter(|ch| !ch.is_control())
        .collect()
}
