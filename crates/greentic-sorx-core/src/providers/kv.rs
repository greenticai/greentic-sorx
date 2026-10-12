//! Canonical-store operations written once against an ordered key-value
//! transaction.
//!
//! The keyspace is byte-for-byte the one [`FoundationDbStore`] documents
//! (`sorx/{tenant}/{sor}/e/{collection}/{id}`, `…/idem/…`, `…/uniq/…`,
//! `…/ev/{stream}/{seq:020}`, `…/evseq/{stream}`, `…/xref/…`, `…/evid/…`,
//! `…/meta/schema_version`, `…/migrations/{id}`), so data written by one
//! backend is readable by the other. A backend supplies [`KvTxn`]: point reads,
//! writes and clears, and an ordered range scan. Everything a caller does
//! inside one [`KvTxn`] must be atomic — the unique-index and idempotency
//! guarantees depend on it.
//!
//! [`FoundationDbStore`]: super::FoundationDbStore

use std::cmp::Ordering;

use serde_json::{Map, Value, json};

use crate::migration::runner::AppliedMigrations;
use crate::{
    AppendEventOp, CreateOp, DeleteOp, DeleteResult, EntityRecord, EventRecord, ExternalRef, GetOp,
    ProviderNamespace, QueryOp, QueryOrder, QueryOrderDirection, SorxError, SorxResult,
    StoreEvidenceOp, UniqueConflictBehavior, UniqueIndex, UpdateOp,
};

/// One atomic unit of work against an ordered byte-keyed store.
pub trait KvTxn {
    fn get(&mut self, key: &[u8]) -> SorxResult<Option<Vec<u8>>>;
    fn set(&mut self, key: &[u8], value: &[u8]) -> SorxResult<()>;
    fn clear(&mut self, key: &[u8]) -> SorxResult<()>;
    /// Every `(key, value)` with `start <= key < end`, ordered by key.
    fn scan(&mut self, start: &[u8], end: &[u8]) -> SorxResult<Vec<(Vec<u8>, Vec<u8>)>>;
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

fn join(namespace: &ProviderNamespace, segments: &[&str]) -> Vec<u8> {
    let mut key = namespace.key_prefix().into_bytes();
    for segment in segments {
        key.push(b'/');
        key.extend_from_slice(segment.as_bytes());
    }
    key
}

pub(crate) fn entity_key(namespace: &ProviderNamespace, collection: &str, id: &str) -> Vec<u8> {
    join(namespace, &["e", &clean_key(collection), &clean_key(id)])
}

fn idem_key(namespace: &ProviderNamespace, collection: &str, key: &str) -> Vec<u8> {
    join(
        namespace,
        &["idem", &clean_key(collection), &clean_key(key)],
    )
}

fn uniq_key(
    namespace: &ProviderNamespace,
    collection: &str,
    index_id: &str,
    values: &[Value],
) -> Vec<u8> {
    join(
        namespace,
        &[
            "uniq",
            &clean_key(collection),
            &clean_key(index_id),
            &encode_values(values),
        ],
    )
}

fn event_key(namespace: &ProviderNamespace, stream: &str, seq: u64) -> Vec<u8> {
    join(
        namespace,
        &["ev", &clean_key(stream), &format!("{seq:020}")],
    )
}

/// Per-collection auto-id counter. Not in the FoundationDB layout, which
/// derives an id from the live record count and so reuses the id of a record
/// after an earlier one is deleted — overwriting it. See [`next_auto_id`].
fn idseq_key(namespace: &ProviderNamespace, collection: &str) -> Vec<u8> {
    join(namespace, &["idseq", &clean_key(collection)])
}

fn evseq_key(namespace: &ProviderNamespace, stream: &str) -> Vec<u8> {
    join(namespace, &["evseq", &clean_key(stream)])
}

fn subject_key(
    namespace: &ProviderNamespace,
    kind: &str,
    collection: &str,
    entity: &str,
    id: &str,
) -> Vec<u8> {
    join(
        namespace,
        &[
            kind,
            &clean_key(collection),
            &clean_key(entity),
            &clean_key(id),
        ],
    )
}

fn schema_key(namespace: &ProviderNamespace) -> Vec<u8> {
    join(namespace, &["meta", "schema_version"])
}

fn migration_key(namespace: &ProviderNamespace, migration_id: &str) -> Vec<u8> {
    join(namespace, &["migrations", &clean_key(migration_id)])
}

/// The `[start, end)` range covering every key strictly UNDER `segments`
/// (a trailing `/` is appended, so `e/order` never matches `e/orders`).
fn child_range(namespace: &ProviderNamespace, segments: &[&str]) -> (Vec<u8>, Vec<u8>) {
    let mut start = join(namespace, segments);
    start.push(b'/');
    let end = prefix_end(&start);
    (start, end)
}

/// Exclusive end key for a prefix range scan (`strinc`), identical to the
/// FoundationDB store's.
pub(crate) fn prefix_end(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.last().copied() {
        if last == 0xff {
            end.pop();
        } else {
            *end.last_mut().expect("non-empty") = last + 1;
            return end;
        }
    }
    vec![0xff]
}

fn encode_values(values: &[Value]) -> String {
    clean_key(&serde_json::to_string(values).unwrap_or_default())
}

pub(crate) fn clean_key(value: &str) -> String {
    value
        .trim_matches('/')
        .replace(['/', '\\'], "_")
        .chars()
        .filter(|ch| !ch.is_control())
        .collect()
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

pub(crate) fn create(txn: &mut impl KvTxn, op: &CreateOp) -> SorxResult<EntityRecord> {
    let namespace = &op.namespace;
    let collection = &op.collection;
    write_schema_version(txn, namespace)?;

    if let Some(key) = &op.idempotency_key
        && let Some(bytes) = txn.get(&idem_key(namespace, collection, key))?
    {
        return decode_json(&bytes);
    }

    let id = match value_id(&op.input) {
        Some(id) => id,
        None => next_auto_id(txn, namespace, collection)?,
    };
    let mut data = object_clone(&op.input);
    data.insert("id".to_string(), Value::String(id.clone()));
    let data = Value::Object(data);

    if let Some(conflict) =
        find_unique_conflict(txn, namespace, collection, &op.unique_indexes, &data, None)?
    {
        if let UniqueConflictBehavior::ReturnExisting { index, fields } = &op.unique_behavior
            && conflict.index.id == *index
            && conflict.index.fields == *fields
        {
            return Ok(conflict.record);
        }
        return Err(unique_conflict_error(&conflict.index, &conflict.values));
    }

    let record = EntityRecord {
        entity: op.entity.clone(),
        collection: collection.clone(),
        id: id.clone(),
        data,
        version: 1,
    };
    let encoded = encode_json(&record)?;
    txn.set(&entity_key(namespace, collection, &id), &encoded)?;
    if let Some(key) = &op.idempotency_key {
        txn.set(&idem_key(namespace, collection, key), &encoded)?;
    }
    write_unique_keys(txn, namespace, collection, &op.unique_indexes, &record)?;
    Ok(record)
}

pub(crate) fn get(txn: &mut impl KvTxn, op: &GetOp) -> SorxResult<Option<EntityRecord>> {
    txn.get(&entity_key(&op.namespace, &op.collection, &op.id))?
        .map(|bytes| decode_json(&bytes))
        .transpose()
}

pub(crate) fn update(txn: &mut impl KvTxn, op: &UpdateOp) -> SorxResult<EntityRecord> {
    let namespace = &op.namespace;
    let collection = &op.collection;
    let ekey = entity_key(namespace, collection, &op.id);
    let existing = txn
        .get(&ekey)?
        .ok_or_else(|| record_not_found(&op.id, collection))?;
    let mut record: EntityRecord = decode_json(&existing)?;

    let mut data = object_clone(&record.data);
    for (key, value) in object_clone(&op.patch) {
        data.insert(key, value);
    }
    data.insert("id".to_string(), Value::String(op.id.clone()));
    let data = Value::Object(data);

    if let Some(conflict) = find_unique_conflict(
        txn,
        namespace,
        collection,
        &op.unique_indexes,
        &data,
        Some(&op.id),
    )? {
        return Err(unique_conflict_error(&conflict.index, &conflict.values));
    }

    // Unlike the FoundationDB store, release the value a changed unique field
    // held: otherwise its entry keeps pointing at this id and a later record
    // taking that value is refused as a conflict. ONLY the indexes this update
    // names, and only when their value changed and the entry is still this
    // record's: an update that names no indexes (a migration backfill, a
    // manager submit) must leave every other index's protection in place.
    for index in &op.unique_indexes {
        let old = index_values(&record.data, &index.fields);
        let new = index_values(&data, &index.fields);
        if let Some(old) = old
            && Some(&old) != new.as_ref()
        {
            let old_key = uniq_key(namespace, collection, &index.id, &old);
            if txn.get(&old_key)?.as_deref() == Some(record.id.as_bytes()) {
                txn.clear(&old_key)?;
            }
        }
    }
    record.data = data;
    record.version += 1;
    txn.set(&ekey, &encode_json(&record)?)?;
    write_unique_keys(txn, namespace, collection, &op.unique_indexes, &record)?;
    Ok(record)
}

pub(crate) fn query_records(txn: &mut impl KvTxn, op: &QueryOp) -> SorxResult<Vec<EntityRecord>> {
    let filter = object_clone(&op.filter);
    let mut records: Vec<EntityRecord> = scan_entities(txn, &op.namespace, &op.collection)?
        .into_iter()
        .filter(|record| matches_filter(&record.data, &filter))
        .collect();
    if op.order_by.is_empty() {
        records.sort_by(|left, right| left.id.cmp(&right.id));
    } else {
        records.sort_by(|left, right| compare_records(left, right, &op.order_by));
    }
    Ok(records)
}

pub(crate) fn delete(txn: &mut impl KvTxn, op: &DeleteOp) -> SorxResult<DeleteResult> {
    let ekey = entity_key(&op.namespace, &op.collection, &op.id);
    let Some(bytes) = txn.get(&ekey)? else {
        return Ok(DeleteResult { deleted: false });
    };
    let record: EntityRecord = decode_json(&bytes)?;
    txn.clear(&ekey)?;
    clear_unique_for_record(txn, &op.namespace, &op.collection, &record.id)?;
    Ok(DeleteResult { deleted: true })
}

pub(crate) fn append_event(txn: &mut impl KvTxn, op: &AppendEventOp) -> SorxResult<EventRecord> {
    let namespace = &op.namespace;
    write_schema_version(txn, namespace)?;
    let seq_key = evseq_key(namespace, &op.stream);
    let last = match txn.get(&seq_key)? {
        Some(bytes) => decode_u64(&bytes)?,
        None => 0,
    };
    let sequence = last + 1;
    let event_id = format!("{}-{}", clean_key(&op.stream), sequence);
    let envelope = json!({
        "event_id": event_id.clone(),
        "event_type": op.event_type.clone(),
        "capability": op.capability.clone(),
        "producer": op.producer.clone(),
        "tenant": namespace.tenant_id.clone(),
        "subject": { "type": op.subject_entity.clone(), "id": op.subject_id.clone() },
        "payload": op.data.clone()
    });
    let record = EventRecord {
        event_id,
        stream: op.stream.clone(),
        event_type: op.event_type.clone(),
        subject_entity: op.subject_entity.clone(),
        subject_id: op.subject_id.clone(),
        data: op.data.clone(),
        envelope,
        sequence,
        occurred_at: op.occurred_at,
    };
    txn.set(
        &event_key(namespace, &op.stream, sequence),
        &encode_json(&record)?,
    )?;
    txn.set(&seq_key, &sequence.to_le_bytes())?;
    Ok(record)
}

pub(crate) fn get_external_refs(
    txn: &mut impl KvTxn,
    namespace: &ProviderNamespace,
    collection: &str,
    entity: &str,
    id: &str,
) -> SorxResult<Vec<ExternalRef>> {
    txn.get(&subject_key(namespace, "xref", collection, entity, id))?
        .map(|bytes| decode_json(&bytes))
        .transpose()
        .map(Option::unwrap_or_default)
}

pub(crate) fn store_evidence(txn: &mut impl KvTxn, op: &StoreEvidenceOp) -> SorxResult<()> {
    let key = subject_key(&op.namespace, "evid", &op.collection, &op.entity, &op.id);
    let mut existing: Vec<Value> = txn
        .get(&key)?
        .map(|bytes| decode_json(&bytes))
        .transpose()?
        .unwrap_or_default();
    existing.push(op.evidence.clone());
    txn.set(&key, &encode_json(&existing)?)
}

pub(crate) fn get_evidence(
    txn: &mut impl KvTxn,
    namespace: &ProviderNamespace,
    collection: &str,
    entity: &str,
    id: &str,
) -> SorxResult<Vec<Value>> {
    txn.get(&subject_key(namespace, "evid", collection, entity, id))?
        .map(|bytes| decode_json(&bytes))
        .transpose()
        .map(Option::unwrap_or_default)
}

pub(crate) fn record_migration(
    txn: &mut impl KvTxn,
    namespace: &ProviderNamespace,
    migration_id: &str,
) -> SorxResult<()> {
    txn.set(
        &migration_key(namespace, migration_id),
        migration_id.as_bytes(),
    )
}

pub(crate) fn load_migrations(
    txn: &mut impl KvTxn,
    namespace: &ProviderNamespace,
) -> SorxResult<AppliedMigrations> {
    let (start, end) = child_range(namespace, &["migrations"]);
    let mut applied = AppliedMigrations::default();
    for (_, value) in txn.scan(&start, &end)? {
        // Strict: a lossy decode would record a mangled id, and the real
        // migration would then run a second time.
        let id = String::from_utf8(value).map_err(|err| {
            SorxError::new(
                "provider_decode_failed",
                format!("a migration ledger entry is not UTF-8: {err}"),
            )
        })?;
        applied.record(&id);
    }
    Ok(applied)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mark the namespace as schema v1, writing only when the marker is absent.
///
/// An unconditional write would make every create and event append in a
/// namespace update the SAME row, serializing all writers on its lock: a
/// throughput ceiling, and a hidden second mechanism behind the unique-index
/// guarantee that the transaction isolation is meant to provide on its own.
fn write_schema_version(txn: &mut impl KvTxn, namespace: &ProviderNamespace) -> SorxResult<()> {
    let key = schema_key(namespace);
    if txn.get(&key)?.as_deref() == Some(b"1".as_slice()) {
        return Ok(());
    }
    txn.set(&key, b"1")
}

/// The next `{collection}-{n}` id no live record holds.
///
/// A counter, not the record count: after a delete the count drops, and a
/// count-derived id lands on a record that still exists, which `set` then
/// overwrites. The counter is seeded from the count the first time, so a
/// collection written by an older build continues where it was; ids already
/// taken (an explicit id, or data written before the counter) are skipped.
fn next_auto_id(
    txn: &mut impl KvTxn,
    namespace: &ProviderNamespace,
    collection: &str,
) -> SorxResult<String> {
    let counter = idseq_key(namespace, collection);
    let mut n = match txn.get(&counter)? {
        Some(bytes) => decode_u64(&bytes)?,
        None => scan_entities(txn, namespace, collection)?.len() as u64,
    };
    loop {
        n += 1;
        let id = format!("{collection}-{n}");
        if txn.get(&entity_key(namespace, collection, &id))?.is_none() {
            txn.set(&counter, &n.to_le_bytes())?;
            return Ok(id);
        }
    }
}

fn scan_entities(
    txn: &mut impl KvTxn,
    namespace: &ProviderNamespace,
    collection: &str,
) -> SorxResult<Vec<EntityRecord>> {
    let (start, end) = child_range(namespace, &["e", &clean_key(collection)]);
    txn.scan(&start, &end)?
        .iter()
        .map(|(_, value)| decode_json(value))
        .collect()
}

struct UniqueConflict {
    index: UniqueIndex,
    values: Vec<Value>,
    record: EntityRecord,
}

fn find_unique_conflict(
    txn: &mut impl KvTxn,
    namespace: &ProviderNamespace,
    collection: &str,
    indexes: &[UniqueIndex],
    data: &Value,
    current_id: Option<&str>,
) -> SorxResult<Option<UniqueConflict>> {
    for index in indexes {
        let Some(values) = index_values(data, &index.fields) else {
            continue;
        };
        let Some(owner) = txn.get(&uniq_key(namespace, collection, &index.id, &values))? else {
            continue;
        };
        let owner = String::from_utf8(owner).map_err(|err| {
            SorxError::new(
                "provider_decode_failed",
                format!("a unique-index entry does not hold a UTF-8 id: {err}"),
            )
        })?;
        if current_id == Some(owner.as_str()) {
            continue;
        }
        if let Some(bytes) = txn.get(&entity_key(namespace, collection, &owner))? {
            return Ok(Some(UniqueConflict {
                index: index.clone(),
                values,
                record: decode_json(&bytes)?,
            }));
        }
    }
    Ok(None)
}

fn write_unique_keys(
    txn: &mut impl KvTxn,
    namespace: &ProviderNamespace,
    collection: &str,
    indexes: &[UniqueIndex],
    record: &EntityRecord,
) -> SorxResult<()> {
    for index in indexes {
        if let Some(values) = index_values(&record.data, &index.fields) {
            txn.set(
                &uniq_key(namespace, collection, &index.id, &values),
                record.id.as_bytes(),
            )?;
        }
    }
    Ok(())
}

fn clear_unique_for_record(
    txn: &mut impl KvTxn,
    namespace: &ProviderNamespace,
    collection: &str,
    id: &str,
) -> SorxResult<()> {
    let (start, end) = child_range(namespace, &["uniq", &clean_key(collection)]);
    for (key, value) in txn.scan(&start, &end)? {
        if value == id.as_bytes() {
            txn.clear(&key)?;
        }
    }
    Ok(())
}

fn encode_json<T: serde::Serialize>(value: &T) -> SorxResult<Vec<u8>> {
    serde_json::to_vec(value)
        .map_err(|err| SorxError::new("provider_encode_failed", err.to_string()))
}

fn decode_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> SorxResult<T> {
    serde_json::from_slice(bytes)
        .map_err(|err| SorxError::new("provider_decode_failed", err.to_string()))
}

/// A counter is exactly eight little-endian bytes. Anything else is refused:
/// reading a damaged counter as 0 would restart the sequence and overwrite the
/// events (or records) it already numbered.
fn decode_u64(bytes: &[u8]) -> SorxResult<u64> {
    let buf: [u8; 8] = bytes.try_into().map_err(|_| {
        SorxError::new(
            "provider_decode_failed",
            format!("a sequence counter is {} bytes, not 8", bytes.len()),
        )
    })?;
    Ok(u64::from_le_bytes(buf))
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
        (Some(left), Some(right)) => serde_json::to_string(left)
            .unwrap_or_default()
            .cmp(&serde_json::to_string(right).unwrap_or_default()),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
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

fn record_not_found(id: &str, collection: &str) -> SorxError {
    SorxError::new(
        "record_not_found",
        format!("record `{id}` was not found in `{collection}`"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns() -> ProviderNamespace {
        ProviderNamespace {
            tenant_id: "acme".into(),
            sor_name: "crm".into(),
        }
    }

    #[test]
    fn entity_keys_match_the_documented_foundationdb_layout() {
        assert_eq!(
            entity_key(&ns(), "orders", "o/1"),
            b"sorx/acme/crm/e/orders/o_1".to_vec()
        );
    }

    #[test]
    fn a_collection_range_does_not_reach_a_collection_sharing_its_prefix() {
        let (start, end) = child_range(&ns(), &["e", "order"]);
        let sibling = entity_key(&ns(), "orders", "1");
        let own = entity_key(&ns(), "order", "1");
        assert!(own >= start && own < end);
        assert!(!(sibling >= start && sibling < end));
    }

    #[test]
    fn prefix_end_increments_and_carries_over_0xff() {
        assert_eq!(prefix_end(b"ab"), b"ac".to_vec());
        assert_eq!(prefix_end(&[b'a', 0xff]), b"b".to_vec());
        assert_eq!(prefix_end(&[0xff, 0xff]), vec![0xff]);
    }
}
