use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    AppendEventOp, CreateOp, DeleteOp, DeleteResult, EntityRecord, EventRecord, EvidenceResult,
    ExternalRef, ExternalRefsOp, ExternalRefsResult, GetOp, IndexQueryOp, IndexQueryResult,
    QueryOp, QueryOrder, QueryOrderDirection, QueryResult, SorStoreProvider, SorxCanonicalStore,
    SorxError, SorxResult, StoreEvidenceOp, TraverseOp, TraverseResult, UniqueConflictBehavior,
    UniqueIndex, UpdateOp,
};

#[derive(Debug, Default)]
pub struct MemoryStoreProvider {
    state: Mutex<MemoryState>,
    persistence_path: Option<PathBuf>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MemoryState {
    collections: BTreeMap<String, BTreeMap<String, EntityRecord>>,
    idempotency: BTreeMap<String, EntityRecord>,
    events: BTreeMap<String, Vec<EventRecord>>,
    external_refs: BTreeMap<String, Vec<ExternalRef>>,
    evidence: BTreeMap<String, Vec<Value>>,
}

/// Current on-disk schema version for the persistent store.
///
/// Bump this whenever the on-disk shape changes and register a matching
/// `migrate_vN_to_vN1` step below.
const CURRENT_SCHEMA_VERSION: u32 = 1;

/// On-disk envelope wrapping the store state with a schema version so the
/// loader can migrate older formats forward on open.
///
/// The persisted file shape is `{ "schema_version": N, "state": { .. } }`.
/// Bare (un-enveloped) [`MemoryState`] files written before versioning carry no
/// `schema_version` key and are treated as schema v0; see [`load_state`] for the
/// detection logic and [`MemoryStoreProvider::persist`] for the writer.
///
/// This typed view is used by the writer to serialize the envelope. The reader
/// works at the [`Value`] level instead, because it must inspect
/// `schema_version` before it knows which historical state shape to decode.
#[derive(Debug, Serialize)]
struct PersistedStore<'a> {
    schema_version: u32,
    state: &'a MemoryState,
}

impl MemoryStoreProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn persistent(path: impl Into<PathBuf>) -> SorxResult<Self> {
        let path = path.into();
        let provider = Self {
            state: Mutex::new(MemoryState::default()),
            persistence_path: Some(path.clone()),
        };
        if !path.exists() {
            return Ok(provider);
        }

        let raw = fs::read_to_string(&path).map_err(|err| {
            SorxError::new(
                "provider_read_failed",
                format!("failed to read persistent store {}: {err}", path.display()),
            )
        })?;
        let loaded = load_state(&raw).map_err(|err| {
            // Preserve the originating error code/path while annotating the file.
            SorxError::new(
                err.code,
                format!("{} (store {})", err.message, path.display()),
            )
        })?;
        {
            let mut guard = provider.state.lock().map_err(lock_error)?;
            *guard = loaded.state;
            // After upgrading an older on-disk format, immediately rewrite the
            // file in the new envelope shape so it loads cleanly next time. A
            // file already at the current version is left untouched (no churn).
            if loaded.migrated {
                provider.persist(&guard)?;
            }
        }
        Ok(provider)
    }

    fn persist(&self, state: &MemoryState) -> SorxResult<()> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                SorxError::new(
                    "provider_write_failed",
                    format!(
                        "failed to create persistent store dir {}: {err}",
                        parent.display()
                    ),
                )
            })?;
        }
        let envelope = PersistedStore {
            schema_version: CURRENT_SCHEMA_VERSION,
            state,
        };
        let encoded = serde_json::to_string_pretty(&envelope)
            .map_err(|err| SorxError::new("provider_encode_failed", err.to_string()))?;
        fs::write(path, format!("{encoded}\n")).map_err(|err| {
            SorxError::new(
                "provider_write_failed",
                format!("failed to write persistent store {}: {err}", path.display()),
            )
        })
    }
}

/// Outcome of decoding (and possibly migrating) a persistent store file.
struct LoadedState {
    state: MemoryState,
    /// `true` when the on-disk version was older than [`CURRENT_SCHEMA_VERSION`]
    /// and the state was migrated forward, signalling the loader to rewrite the
    /// file in the current envelope format.
    migrated: bool,
}

/// Decode the raw persistent-store contents, detecting the on-disk schema
/// version and migrating older formats forward.
///
/// Version detection: the raw JSON is parsed as a [`Value`] first. If it is an
/// object carrying a numeric `schema_version` field it is treated as an
/// envelope ([`PersistedStore`]) and that field is the source version. Any other
/// object is a legacy pre-versioning bare [`MemoryState`] and is treated as
/// schema v0, with the whole object taken as the state value.
fn load_state(raw: &str) -> SorxResult<LoadedState> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|err| SorxError::new("provider_decode_failed", err.to_string()))?;

    let (from_version, state_value) = match value
        .as_object()
        .and_then(|object| object.get("schema_version"))
        .and_then(Value::as_u64)
    {
        Some(version) => {
            let version = u32::try_from(version).map_err(|_| {
                SorxError::new(
                    "provider_schema_too_new",
                    format!(
                        "store schema v{version} is newer than supported v{CURRENT_SCHEMA_VERSION} \
                         — upgrade the runtime"
                    ),
                )
            })?;
            let state_value = value
                .get("state")
                .cloned()
                .unwrap_or(Value::Object(Map::new()));
            (version, state_value)
        }
        // No `schema_version` key: a legacy bare MemoryState (schema v0).
        None => (0, value),
    };

    if from_version > CURRENT_SCHEMA_VERSION {
        return Err(SorxError::new(
            "provider_schema_too_new",
            format!(
                "store schema v{from_version} is newer than supported v{CURRENT_SCHEMA_VERSION} \
                 — upgrade the runtime"
            ),
        ));
    }

    let state = migrate(from_version, state_value)?;
    Ok(LoadedState {
        state,
        migrated: from_version < CURRENT_SCHEMA_VERSION,
    })
}

/// Stepwise migration executor.
///
/// Each `migrate_vN_to_vN1` is a `Value` -> `Value` transform on the raw store
/// state JSON. Keeping migrations at the `Value` level (rather than the typed
/// [`MemoryState`]) lets future steps add, rename, or drop fields without
/// needing the historical struct shape to still compile. The single typed
/// decode into [`MemoryState`] happens once, at the end, after all steps have
/// run.
fn migrate(from: u32, mut state_value: Value) -> SorxResult<MemoryState> {
    let mut version = from;
    while version < CURRENT_SCHEMA_VERSION {
        state_value = match version {
            0 => migrate_v0_to_v1(state_value)?,
            other => {
                return Err(SorxError::new(
                    "provider_migration_missing",
                    format!("no migration from schema v{other}"),
                ));
            }
        };
        version += 1;
    }
    serde_json::from_value(state_value)
        .map_err(|err| SorxError::new("provider_decode_failed", err.to_string()))
}

/// Migrate a schema v0 (bare, pre-versioning) store state to schema v1.
///
/// v0 and v1 share the exact same state shape: v1 only adds the on-disk
/// envelope (the `schema_version`/`state` wrapper) around an otherwise
/// identical [`MemoryState`]. The version bump therefore records that the file
/// is now envelope-wrapped; the state itself is carried through unchanged. This
/// step is kept as a real registered migration so the executor is exercised and
/// future non-identity steps slot in beside it.
fn migrate_v0_to_v1(state_value: Value) -> SorxResult<Value> {
    Ok(state_value)
}

impl SorStoreProvider for MemoryStoreProvider {
    fn create(&self, op: CreateOp) -> SorxResult<EntityRecord> {
        let mut state = self.state.lock().map_err(lock_error)?;
        let collection_key = collection_key(&op.namespace, &op.collection);
        let idempotency_key = op
            .idempotency_key
            .as_ref()
            .map(|key| format!("{collection_key}:{key}"));
        if let Some(key) = &idempotency_key
            && let Some(record) = state.idempotency.get(key)
        {
            return Ok(record.clone());
        }

        let collection_len = state
            .collections
            .get(&collection_key)
            .map(BTreeMap::len)
            .unwrap_or(0);
        let id = value_id(&op.input)
            .unwrap_or_else(|| format!("{}-{}", op.collection, collection_len + 1));
        let mut data = object_clone(&op.input);
        data.insert("id".to_string(), Value::String(id.clone()));
        if let Some(conflict) = unique_conflict(
            state.collections.get(&collection_key),
            &op.unique_indexes,
            &Value::Object(data.clone()),
            None,
        ) {
            if let UniqueConflictBehavior::ReturnExisting { index, fields } = &op.unique_behavior
                && conflict.index.id == *index
                && conflict.index.fields == *fields
            {
                return Ok(conflict.record);
            }
            return Err(unique_conflict_error(&conflict.index, &conflict.values));
        }

        let record = EntityRecord {
            entity: op.entity,
            collection: op.collection.clone(),
            id: id.clone(),
            data: Value::Object(data),
            version: 1,
        };
        state
            .collections
            .entry(collection_key)
            .or_default()
            .insert(id, record.clone());
        if let Some(key) = idempotency_key {
            state.idempotency.insert(key, record.clone());
        }
        self.persist(&state)?;
        Ok(record)
    }

    fn get(&self, op: GetOp) -> SorxResult<Option<EntityRecord>> {
        let state = self.state.lock().map_err(lock_error)?;
        Ok(state
            .collections
            .get(&collection_key(&op.namespace, &op.collection))
            .and_then(|collection| collection.get(&op.id))
            .cloned())
    }

    fn update(&self, op: UpdateOp) -> SorxResult<EntityRecord> {
        let mut state = self.state.lock().map_err(lock_error)?;
        let collection = state
            .collections
            .get_mut(&collection_key(&op.namespace, &op.collection))
            .ok_or_else(|| {
                SorxError::new(
                    "record_not_found",
                    format!("record `{}` was not found in `{}`", op.id, op.collection),
                )
            })?;
        let existing = collection.get(&op.id).ok_or_else(|| {
            SorxError::new(
                "record_not_found",
                format!("record `{}` was not found in `{}`", op.id, op.collection),
            )
        })?;
        let mut data = object_clone(&existing.data);
        for (key, value) in object_clone(&op.patch) {
            data.insert(key, value);
        }
        data.insert("id".to_string(), Value::String(op.id.clone()));
        if let Some(conflict) = unique_conflict(
            Some(collection),
            &op.unique_indexes,
            &Value::Object(data.clone()),
            Some(&op.id),
        ) {
            return Err(unique_conflict_error(&conflict.index, &conflict.values));
        }
        let record = collection.get_mut(&op.id).ok_or_else(|| {
            SorxError::new(
                "record_not_found",
                format!("record `{}` was not found in `{}`", op.id, op.collection),
            )
        })?;
        record.data = Value::Object(data);
        record.version += 1;
        let record = record.clone();
        self.persist(&state)?;
        Ok(record)
    }

    fn query(&self, op: QueryOp) -> SorxResult<QueryResult> {
        let state = self.state.lock().map_err(lock_error)?;
        let filter = object_clone(&op.filter);
        let mut records = state
            .collections
            .get(&collection_key(&op.namespace, &op.collection))
            .map(|collection| {
                collection
                    .values()
                    .filter(|record| matches_filter(&record.data, &filter))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if op.order_by.is_empty() {
            records.sort_by(|left, right| left.id.cmp(&right.id));
        } else {
            records.sort_by(|left, right| compare_records(left, right, &op.order_by));
        }
        Ok(QueryResult { records })
    }

    fn delete(&self, op: DeleteOp) -> SorxResult<DeleteResult> {
        let mut state = self.state.lock().map_err(lock_error)?;
        let deleted = state
            .collections
            .get_mut(&collection_key(&op.namespace, &op.collection))
            .and_then(|collection| collection.remove(&op.id))
            .is_some();
        self.persist(&state)?;
        Ok(DeleteResult { deleted })
    }
}

impl SorxCanonicalStore for MemoryStoreProvider {
    fn append_event(&self, op: AppendEventOp) -> SorxResult<EventRecord> {
        let mut state = self.state.lock().map_err(lock_error)?;
        let stream_key = format!(
            "{}/events/{}",
            op.namespace.key_prefix(),
            clean_key(&op.stream)
        );
        let stream = state.events.entry(stream_key).or_default();
        let sequence = stream.len() as u64 + 1;
        let event_id = format!("{}-{}", clean_key(&op.stream), sequence);
        let envelope = json!({
            "event_id": event_id.clone(),
            "event_type": op.event_type.clone(),
            "capability": op.capability.clone(),
            "producer": op.producer.clone(),
            "tenant": op.namespace.tenant_id.clone(),
            "subject": {
                "type": op.subject_entity.clone(),
                "id": op.subject_id.clone()
            },
            "payload": op.data.clone()
        });
        let record = EventRecord {
            event_id,
            stream: op.stream,
            event_type: op.event_type,
            subject_entity: op.subject_entity,
            subject_id: op.subject_id,
            data: op.data,
            envelope,
            sequence,
        };
        stream.push(record.clone());
        self.persist(&state)?;
        Ok(record)
    }

    fn query_index(&self, op: IndexQueryOp) -> SorxResult<IndexQueryResult> {
        let query = self.query(QueryOp {
            namespace: op.namespace,
            entity: op.entity,
            collection: op.collection,
            filter: op.filter,
            order_by: Vec::new(),
        })?;
        Ok(IndexQueryResult {
            records: query.records,
        })
    }

    fn traverse(&self, op: TraverseOp) -> SorxResult<TraverseResult> {
        let record = self.get(GetOp {
            namespace: op.namespace,
            entity: op.root_entity,
            collection: op.root_collection,
            id: op.root_id,
        })?;
        Ok(TraverseResult {
            records: record.into_iter().collect(),
        })
    }

    fn get_external_refs(&self, op: ExternalRefsOp) -> SorxResult<ExternalRefsResult> {
        let state = self.state.lock().map_err(lock_error)?;
        Ok(ExternalRefsResult {
            refs: state
                .external_refs
                .get(&subject_key(
                    &op.namespace,
                    &op.collection,
                    &op.entity,
                    &op.id,
                ))
                .cloned()
                .unwrap_or_default(),
        })
    }

    fn store_evidence(&self, op: StoreEvidenceOp) -> SorxResult<()> {
        let mut state = self.state.lock().map_err(lock_error)?;
        let key = subject_key(&op.namespace, &op.collection, &op.entity, &op.id);
        state.evidence.entry(key).or_default().push(op.evidence);
        self.persist(&state)?;
        Ok(())
    }

    fn get_evidence(&self, op: ExternalRefsOp) -> SorxResult<EvidenceResult> {
        let state = self.state.lock().map_err(lock_error)?;
        Ok(EvidenceResult {
            evidence: state
                .evidence
                .get(&subject_key(
                    &op.namespace,
                    &op.collection,
                    &op.entity,
                    &op.id,
                ))
                .cloned()
                .unwrap_or_default(),
        })
    }
}

fn collection_key(namespace: &crate::ProviderNamespace, collection: &str) -> String {
    format!("{}/{collection}", namespace.key_prefix())
}

struct UniqueConflict {
    index: UniqueIndex,
    values: Vec<Value>,
    record: EntityRecord,
}

fn unique_conflict(
    collection: Option<&BTreeMap<String, EntityRecord>>,
    indexes: &[UniqueIndex],
    data: &Value,
    current_id: Option<&str>,
) -> Option<UniqueConflict> {
    let collection = collection?;
    for index in indexes {
        let values = index_values(data, &index.fields)?;
        for record in collection.values() {
            if current_id.is_some_and(|id| record.id == id) {
                continue;
            }
            if index_values(&record.data, &index.fields).as_ref() == Some(&values) {
                return Some(UniqueConflict {
                    index: index.clone(),
                    values,
                    record: record.clone(),
                });
            }
        }
    }
    None
}

fn index_values(data: &Value, fields: &[String]) -> Option<Vec<Value>> {
    fields
        .iter()
        .map(|field| lookup_path(data, field).cloned())
        .collect()
}

fn lookup_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn unique_conflict_error(index: &UniqueIndex, values: &[Value]) -> SorxError {
    SorxError::new(
        "unique_constraint_violation",
        format!(
            "unique index `{}` already has fields {:?} values {:?}",
            index.id, index.fields, values
        ),
    )
}

fn subject_key(
    namespace: &crate::ProviderNamespace,
    collection: &str,
    entity: &str,
    id: &str,
) -> String {
    format!(
        "{}/{}/{}/{}",
        namespace.key_prefix(),
        clean_key(collection),
        clean_key(entity),
        clean_key(id)
    )
}

fn clean_key(value: &str) -> String {
    value
        .trim_matches('/')
        .replace(['/', '\\'], "_")
        .chars()
        .filter(|ch| !ch.is_control())
        .collect()
}

fn value_id(value: &Value) -> Option<String> {
    value
        .get("id")
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn object_clone(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn matches_filter(data: &Value, filter: &Map<String, Value>) -> bool {
    filter
        .iter()
        .all(|(key, expected)| data.get(key) == Some(expected))
}

fn compare_records(left: &EntityRecord, right: &EntityRecord, order_by: &[QueryOrder]) -> Ordering {
    for order in order_by {
        let ordering = compare_values(
            lookup_path(&left.data, &order.field),
            lookup_path(&right.data, &order.field),
        );
        let ordering = match order.direction {
            QueryOrderDirection::Asc => ordering,
            QueryOrderDirection::Desc => ordering.reverse(),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.id.cmp(&right.id)
}

fn compare_values(left: Option<&Value>, right: Option<&Value>) -> Ordering {
    match (left, right) {
        (Some(Value::Number(left)), Some(Value::Number(right))) => {
            match (left.as_f64(), right.as_f64()) {
                (Some(left), Some(right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
                _ => Ordering::Equal,
            }
        }
        (Some(Value::String(left)), Some(Value::String(right))) => left.cmp(right),
        (Some(Value::Bool(left)), Some(Value::Bool(right))) => left.cmp(right),
        (Some(left), Some(right)) => value_sort_key(left).cmp(&value_sort_key(right)),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

fn value_sort_key(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> SorxError {
    SorxError::new(
        "provider_lock_failed",
        "in-memory provider lock was poisoned",
    )
}
