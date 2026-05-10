use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{Map, Value};

use crate::{
    CreateOp, DeleteOp, DeleteResult, EntityRecord, GetOp, QueryOp, QueryResult, SorStoreProvider,
    SorxError, SorxResult, UpdateOp,
};

#[derive(Debug, Default)]
pub struct MemoryStoreProvider {
    state: Mutex<MemoryState>,
}

#[derive(Debug, Default)]
struct MemoryState {
    collections: BTreeMap<String, BTreeMap<String, EntityRecord>>,
    idempotency: BTreeMap<String, EntityRecord>,
}

impl MemoryStoreProvider {
    pub fn new() -> Self {
        Self::default()
    }
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
        let record = collection.get_mut(&op.id).ok_or_else(|| {
            SorxError::new(
                "record_not_found",
                format!("record `{}` was not found in `{}`", op.id, op.collection),
            )
        })?;
        let mut data = object_clone(&record.data);
        for (key, value) in object_clone(&op.patch) {
            data.insert(key, value);
        }
        data.insert("id".to_string(), Value::String(op.id));
        record.data = Value::Object(data);
        record.version += 1;
        Ok(record.clone())
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
        records.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(QueryResult { records })
    }

    fn delete(&self, op: DeleteOp) -> SorxResult<DeleteResult> {
        let mut state = self.state.lock().map_err(lock_error)?;
        let deleted = state
            .collections
            .get_mut(&collection_key(&op.namespace, &op.collection))
            .and_then(|collection| collection.remove(&op.id))
            .is_some();
        Ok(DeleteResult { deleted })
    }
}

fn collection_key(namespace: &crate::ProviderNamespace, collection: &str) -> String {
    format!("{}/{collection}", namespace.key_prefix())
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

fn lock_error<T>(_: std::sync::PoisonError<T>) -> SorxError {
    SorxError::new(
        "provider_lock_failed",
        "in-memory provider lock was poisoned",
    )
}
