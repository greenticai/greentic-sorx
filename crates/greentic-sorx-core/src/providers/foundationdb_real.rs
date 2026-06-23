//! Real FoundationDB-backed canonical store (`--features foundationdb`).
//!
//! # Keyspace layout
//!
//! All keys live under the namespace prefix `sorx/{tenant_id}/{sor_name}`
//! (see [`ProviderNamespace::key_prefix`]). Sub-keyspaces, joined with `/`:
//!
//! | purpose       | key                                              | value                |
//! |---------------|--------------------------------------------------|----------------------|
//! | entity        | `<prefix>/e/{collection}/{id}`                   | JSON(`EntityRecord`) |
//! | idempotency   | `<prefix>/idem/{collection}/{key}`               | JSON(`EntityRecord`) |
//! | unique index  | `<prefix>/uniq/{collection}/{index_id}/{values}` | the entity id        |
//! | event         | `<prefix>/ev/{stream}/{seq:020}`                 | JSON(`EventRecord`)  |
//! | event counter | `<prefix>/evseq/{stream}`                         | u64 LE (next seq-1)  |
//! | external refs | `<prefix>/xref/{collection}/{entity}/{id}`       | JSON(`Vec<ExternalRef>`) |
//! | evidence      | `<prefix>/evid/{collection}/{entity}/{id}`       | JSON(`Vec<Value>`)   |
//! | schema/meta   | `<prefix>/meta/schema_version`                   | `"1"`                |
//!
//! Path segments are sanitized via [`clean_key`] (matching the in-memory
//! provider) so a `/` inside a tenant id / collection / id cannot escape its
//! sub-keyspace.
//!
//! # Sync-over-async / network boot
//!
//! The `foundationdb` client boots a global network thread exactly once per
//! process. We guard that with a process-global [`OnceLock`] holding the
//! `NetworkAutoStop` (kept alive for the process lifetime) plus a dedicated
//! multi-thread Tokio runtime used to drive the async FDB futures. The trait
//! methods are synchronous, so each op runs its async body via
//! `Handle::block_on`, wrapped in `block_in_place` when a multi-thread runtime
//! is already current (mirroring the `DesignerLlmBridge` pattern). If the
//! caller is *inside* a current-thread runtime, blocking is not possible and we
//! return a structured error rather than panicking.
//!
//! # Transactions
//!
//! Every op that touches multiple keys (create with unique indexes +
//! idempotency, update, delete clearing index keys, event append + counter)
//! runs inside a single FDB transaction via `Database::run`, so it is atomic —
//! the real durability/consistency win over the in-memory provider.

use std::cmp::Ordering;
use std::sync::OnceLock;

use foundationdb::api::NetworkAutoStop;
use foundationdb::tuple::Subspace;
use foundationdb::{Database, FdbBindingError, KeySelector, RangeOption};
use serde_json::{Map, Value, json};
use tokio::runtime::{Handle, Runtime};

use crate::migration::runner::AppliedMigrations;
use crate::{
    AppendEventOp, CreateOp, DeleteOp, DeleteResult, EntityRecord, EventRecord, EvidenceResult,
    ExternalRef, ExternalRefsOp, ExternalRefsResult, GetOp, IndexQueryOp, IndexQueryResult,
    ProviderNamespace, QueryOp, QueryOrder, QueryOrderDirection, QueryResult, SorStoreProvider,
    SorxCanonicalStore, SorxError, SorxResult, StoreEvidenceOp, TraverseOp, TraverseResult,
    UniqueConflictBehavior, UniqueIndex, UpdateOp,
};

/// Process-global FDB network guard. `NetworkAutoStop` must outlive every
/// transaction; we leak it into a `OnceLock` for the process lifetime.
static FDB_NETWORK: OnceLock<NetworkAutoStop> = OnceLock::new();
/// Dedicated runtime to drive async FDB work from sync trait methods.
static FDB_RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn boot_network() -> SorxResult<()> {
    // `select_api_version` (inside `boot`) may run at most ONCE per process, and
    // concurrent callers must not race it. `OnceLock::get_or_init` serializes
    // initialization so `boot` is invoked exactly once even under parallel test
    // threads. The returned guard MUST live as long as any FDB API is used, so
    // we keep it in the static for the process lifetime.
    FDB_NETWORK.get_or_init(|| {
        // SAFETY: invoked exactly once (guarded by `get_or_init`); the resulting
        // `NetworkAutoStop` is stored in the static and never dropped until the
        // process exits, so no FDB API is used after the network stops.
        unsafe { foundationdb::boot() }
    });
    Ok(())
}

fn runtime() -> SorxResult<&'static Runtime> {
    if let Some(rt) = FDB_RUNTIME.get() {
        return Ok(rt);
    }
    let rt = Runtime::new()
        .map_err(|err| fdb_error(format!("failed to start FoundationDB runtime: {err}")))?;
    // Race-tolerant: if another thread set it first, keep theirs.
    let _ = FDB_RUNTIME.set(rt);
    FDB_RUNTIME
        .get()
        .ok_or_else(|| fdb_error("FoundationDB runtime unavailable"))
}

/// Real FoundationDB-backed store. Holds an open [`Database`] handle.
pub struct FoundationDbStore {
    db: Database,
}

impl std::fmt::Debug for FoundationDbStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FoundationDbStore").finish_non_exhaustive()
    }
}

impl FoundationDbStore {
    /// Connect to a cluster using the given cluster file path. Boots the FDB
    /// network (once per process) and opens a database handle.
    pub fn connect(cluster_file: &str) -> SorxResult<Self> {
        boot_network()?;
        let db = Database::from_path(cluster_file)
            .map_err(|err| fdb_error(format!("failed to open FoundationDB database: {err}")))?;
        Ok(Self { db })
    }

    /// Run an async FDB body on the dedicated runtime, blocking the current
    /// thread. Uses `block_in_place` when a multi-thread runtime is current so
    /// we don't stall its worker threads; errors out from a current-thread
    /// runtime where blocking is impossible.
    fn block_on<F, T>(&self, fut: F) -> SorxResult<T>
    where
        F: std::future::Future<Output = SorxResult<T>>,
    {
        let rt = runtime()?;
        match Handle::try_current() {
            Ok(handle) => {
                if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread {
                    return Err(fdb_error(
                        "FoundationDB store requires a multi-thread Tokio runtime; \
                         it cannot run inside a current-thread runtime",
                    ));
                }
                tokio::task::block_in_place(|| rt.block_on(fut))
            }
            Err(_) => rt.block_on(fut),
        }
    }

    // -----------------------------------------------------------------------
    // Migration ledger
    // -----------------------------------------------------------------------
    // Keyspace: `<ns_prefix>/migrations/<clean(migration_id)>` → migration id
    // A range scan over `<ns_prefix>/migrations/` collects all recorded ids.

    /// Record a migration id as applied under the given namespace.
    ///
    /// Idempotent: calling it twice for the same id is safe.
    pub fn record_migration_applied(
        &self,
        namespace: &ProviderNamespace,
        migration_id: &str,
    ) -> SorxResult<()> {
        let key = migration_key(namespace, migration_id);
        let value = migration_id.as_bytes().to_vec();
        self.block_on(async move {
            self.db
                .run(|trx, _| {
                    let key = key.clone();
                    let value = value.clone();
                    async move {
                        trx.set(&key, &value);
                        Ok(())
                    }
                })
                .await
                .map_err(lower)
        })
    }

    /// Load all migration ids recorded under the given namespace.
    ///
    /// Returns an empty [`AppliedMigrations`] when no migrations have been
    /// recorded yet.
    pub fn load_applied_migrations(
        &self,
        namespace: &ProviderNamespace,
    ) -> SorxResult<AppliedMigrations> {
        let mut prefix = join(&ns_subspace(namespace), &["migrations"]);
        // Trailing separator ensures the scan matches only exact subkeys, not
        // other subspaces that share the `migrations` prefix.
        prefix.push(b'/');
        let end = prefix_end(&prefix);
        self.block_on(async move {
            let kvs = self
                .db
                .run(|trx, _| {
                    let prefix = prefix.clone();
                    let end = end.clone();
                    async move {
                        let range = RangeOption::from((
                            KeySelector::first_greater_or_equal(prefix),
                            KeySelector::first_greater_or_equal(end),
                        ));
                        // `?` converts FdbError -> FdbBindingError via From impl.
                        let values = trx.get_range(&range, 1, false).await?;
                        Ok(values)
                    }
                })
                .await
                .map_err(lower)?;
            let mut applied = AppliedMigrations::default();
            for kv in &kvs {
                let id = String::from_utf8_lossy(kv.value()).to_string();
                applied.record(&id);
            }
            Ok(applied)
        })
    }
}

/// Map any FDB/binding error or layer message into a `SorxError`.
fn fdb_error(message: impl Into<String>) -> SorxError {
    SorxError::new("provider_fdb_error", message.into())
}

/// Lift a `SorxError` out of a transaction closure as an FDB binding error so
/// `Database::run` propagates it without retrying as if it were a DB fault.
fn lift(err: SorxError) -> FdbBindingError {
    FdbBindingError::CustomError(Box::new(err))
}

/// Recover a `SorxError` from a binding error coming back out of `run`.
fn lower(err: FdbBindingError) -> SorxError {
    match err {
        FdbBindingError::CustomError(boxed) => match boxed.downcast::<SorxError>() {
            Ok(sorx) => *sorx,
            Err(other) => fdb_error(other.to_string()),
        },
        other => fdb_error(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Key construction
// ---------------------------------------------------------------------------

fn ns_subspace(namespace: &ProviderNamespace) -> Subspace {
    // The string prefix is reused verbatim as the subspace prefix bytes so the
    // layout matches the documented `sorx/{tenant}/{sor}/...` scheme.
    Subspace::from_bytes(namespace.key_prefix().as_bytes())
}

fn entity_prefix(namespace: &ProviderNamespace, collection: &str) -> Vec<u8> {
    join(&ns_subspace(namespace), &["e", &clean_key(collection)])
}

fn entity_key(namespace: &ProviderNamespace, collection: &str, id: &str) -> Vec<u8> {
    join(
        &ns_subspace(namespace),
        &["e", &clean_key(collection), &clean_key(id)],
    )
}

fn idem_key(namespace: &ProviderNamespace, collection: &str, key: &str) -> Vec<u8> {
    join(
        &ns_subspace(namespace),
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
        &ns_subspace(namespace),
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
        &ns_subspace(namespace),
        &["ev", &clean_key(stream), &format!("{seq:020}")],
    )
}

fn evseq_key(namespace: &ProviderNamespace, stream: &str) -> Vec<u8> {
    join(&ns_subspace(namespace), &["evseq", &clean_key(stream)])
}

fn subject_key(
    namespace: &ProviderNamespace,
    kind: &str,
    collection: &str,
    entity: &str,
    id: &str,
) -> Vec<u8> {
    join(
        &ns_subspace(namespace),
        &[
            kind,
            &clean_key(collection),
            &clean_key(entity),
            &clean_key(id),
        ],
    )
}

fn schema_key(namespace: &ProviderNamespace) -> Vec<u8> {
    join(&ns_subspace(namespace), &["meta", "schema_version"])
}

fn migration_key(namespace: &ProviderNamespace, migration_id: &str) -> Vec<u8> {
    join(
        &ns_subspace(namespace),
        &["migrations", &clean_key(migration_id)],
    )
}

/// Join `/`-separated string segments onto the namespace subspace prefix bytes.
fn join(subspace: &Subspace, segments: &[&str]) -> Vec<u8> {
    let mut key = subspace.bytes().to_vec();
    for segment in segments {
        key.push(b'/');
        key.extend_from_slice(segment.as_bytes());
    }
    key
}

/// Exclusive end key for a prefix range scan (`strinc`).
fn prefix_end(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.last().copied() {
        if last == 0xff {
            end.pop();
        } else {
            *end.last_mut().expect("non-empty") = last + 1;
            return end;
        }
    }
    // All-0xff prefix: range to the very end of the keyspace.
    vec![0xff]
}

/// Stable encoding of index field values for the unique-index key segment.
fn encode_values(values: &[Value]) -> String {
    clean_key(&serde_json::to_string(values).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// SorStoreProvider
// ---------------------------------------------------------------------------

impl SorStoreProvider for FoundationDbStore {
    fn create(&self, op: CreateOp) -> SorxResult<EntityRecord> {
        self.block_on(async move {
            let namespace = op.namespace;
            let collection = op.collection;
            let entity = op.entity;
            let input = op.input;
            let idempotency_key = op.idempotency_key;
            let unique_indexes = op.unique_indexes;
            let unique_behavior = op.unique_behavior;

            self.db
                .run(|trx, _| {
                    let namespace = namespace.clone();
                    let collection = collection.clone();
                    let entity = entity.clone();
                    let input = input.clone();
                    let idempotency_key = idempotency_key.clone();
                    let unique_indexes = unique_indexes.clone();
                    let unique_behavior = unique_behavior.clone();
                    async move {
                        write_schema_version(&trx, &namespace);

                        // Idempotency short-circuit.
                        if let Some(key) = &idempotency_key {
                            let ikey = idem_key(&namespace, &collection, key);
                            if let Some(slice) = trx.get(&ikey, false).await? {
                                let record = decode_record(&slice).map_err(lift)?;
                                return Ok(record);
                            }
                        }

                        // Auto-id: `{collection}-{count+1}` based on existing
                        // entity count (matches the in-memory provider).
                        let id = match value_id(&input) {
                            Some(id) => id,
                            None => {
                                let count = count_entities(&trx, &namespace, &collection).await?;
                                format!("{collection}-{}", count + 1)
                            }
                        };

                        let mut data = object_clone(&input);
                        data.insert("id".to_string(), Value::String(id.clone()));
                        let data_value = Value::Object(data);

                        // Unique-index conflict detection.
                        if let Some(conflict) = find_unique_conflict(
                            &trx,
                            &namespace,
                            &collection,
                            &unique_indexes,
                            &data_value,
                            None,
                        )
                        .await?
                        {
                            if let UniqueConflictBehavior::ReturnExisting { index, fields } =
                                &unique_behavior
                                && conflict.index.id == *index
                                && conflict.index.fields == *fields
                            {
                                return Ok(conflict.record);
                            }
                            return Err(lift(unique_conflict_error(
                                &conflict.index,
                                &conflict.values,
                            )));
                        }

                        let record = EntityRecord {
                            entity: entity.clone(),
                            collection: collection.clone(),
                            id: id.clone(),
                            data: data_value.clone(),
                            version: 1,
                        };

                        // Persist entity, idempotency pointer, and unique index keys.
                        let ekey = entity_key(&namespace, &collection, &id);
                        trx.set(&ekey, &encode_record(&record).map_err(lift)?);
                        if let Some(key) = &idempotency_key {
                            let ikey = idem_key(&namespace, &collection, key);
                            trx.set(&ikey, &encode_record(&record).map_err(lift)?);
                        }
                        for index in &unique_indexes {
                            if let Some(values) = index_values(&data_value, &index.fields) {
                                let ukey = uniq_key(&namespace, &collection, &index.id, &values);
                                trx.set(&ukey, id.as_bytes());
                            }
                        }
                        Ok(record)
                    }
                })
                .await
                .map_err(lower)
        })
    }

    fn get(&self, op: GetOp) -> SorxResult<Option<EntityRecord>> {
        self.block_on(async move {
            let key = entity_key(&op.namespace, &op.collection, &op.id);
            self.db
                .run(|trx, _| {
                    let key = key.clone();
                    async move {
                        match trx.get(&key, false).await? {
                            Some(slice) => Ok(Some(decode_record(&slice).map_err(lift)?)),
                            None => Ok(None),
                        }
                    }
                })
                .await
                .map_err(lower)
        })
    }

    fn update(&self, op: UpdateOp) -> SorxResult<EntityRecord> {
        self.block_on(async move {
            let namespace = op.namespace;
            let collection = op.collection;
            let id = op.id;
            let patch = op.patch;
            let unique_indexes = op.unique_indexes;

            self.db
                .run(|trx, _| {
                    let namespace = namespace.clone();
                    let collection = collection.clone();
                    let id = id.clone();
                    let patch = patch.clone();
                    let unique_indexes = unique_indexes.clone();
                    async move {
                        let ekey = entity_key(&namespace, &collection, &id);
                        let existing = trx
                            .get(&ekey, false)
                            .await?
                            .ok_or_else(|| lift(record_not_found(&id, &collection)))?;
                        let mut record = decode_record(&existing).map_err(lift)?;

                        let mut data = object_clone(&record.data);
                        for (k, v) in object_clone(&patch) {
                            data.insert(k, v);
                        }
                        data.insert("id".to_string(), Value::String(id.clone()));
                        let data_value = Value::Object(data);

                        if let Some(conflict) = find_unique_conflict(
                            &trx,
                            &namespace,
                            &collection,
                            &unique_indexes,
                            &data_value,
                            Some(&id),
                        )
                        .await?
                        {
                            return Err(lift(unique_conflict_error(
                                &conflict.index,
                                &conflict.values,
                            )));
                        }

                        // Refresh unique-index keys for this record's new values.
                        for index in &unique_indexes {
                            if let Some(values) = index_values(&data_value, &index.fields) {
                                let ukey = uniq_key(&namespace, &collection, &index.id, &values);
                                trx.set(&ukey, id.as_bytes());
                            }
                        }

                        record.data = data_value;
                        record.version += 1;
                        trx.set(&ekey, &encode_record(&record).map_err(lift)?);
                        Ok(record)
                    }
                })
                .await
                .map_err(lower)
        })
    }

    fn query(&self, op: QueryOp) -> SorxResult<QueryResult> {
        self.block_on(async move {
            let namespace = op.namespace;
            let collection = op.collection;
            let filter = object_clone(&op.filter);
            let order_by = op.order_by;

            let mut records = self
                .db
                .run(|trx, _| {
                    let namespace = namespace.clone();
                    let collection = collection.clone();
                    let filter = filter.clone();
                    async move {
                        let mut out = Vec::new();
                        for record in scan_entities(&trx, &namespace, &collection).await? {
                            if matches_filter(&record.data, &filter) {
                                out.push(record);
                            }
                        }
                        Ok(out)
                    }
                })
                .await
                .map_err(lower)?;

            if order_by.is_empty() {
                records.sort_by(|left, right| left.id.cmp(&right.id));
            } else {
                records.sort_by(|left, right| compare_records(left, right, &order_by));
            }
            Ok(QueryResult { records })
        })
    }

    fn delete(&self, op: DeleteOp) -> SorxResult<DeleteResult> {
        self.block_on(async move {
            let namespace = op.namespace;
            let collection = op.collection;
            let id = op.id;

            self.db
                .run(|trx, _| {
                    let namespace = namespace.clone();
                    let collection = collection.clone();
                    let id = id.clone();
                    async move {
                        let ekey = entity_key(&namespace, &collection, &id);
                        match trx.get(&ekey, false).await? {
                            Some(slice) => {
                                let record = decode_record(&slice).map_err(lift)?;
                                // Clear the entity plus any unique-index keys
                                // that point at this record's field values.
                                trx.clear(&ekey);
                                clear_unique_for_record(&trx, &namespace, &collection, &record)
                                    .await?;
                                Ok(DeleteResult { deleted: true })
                            }
                            None => Ok(DeleteResult { deleted: false }),
                        }
                    }
                })
                .await
                .map_err(lower)
        })
    }
}

// ---------------------------------------------------------------------------
// SorxCanonicalStore
// ---------------------------------------------------------------------------

impl SorxCanonicalStore for FoundationDbStore {
    fn append_event(&self, op: AppendEventOp) -> SorxResult<EventRecord> {
        self.block_on(async move {
            let namespace = op.namespace;
            let stream = op.stream;
            let event_type = op.event_type;
            let capability = op.capability;
            let producer = op.producer;
            let subject_entity = op.subject_entity;
            let subject_id = op.subject_id;
            let data = op.data;

            self.db
                .run(|trx, _| {
                    let namespace = namespace.clone();
                    let stream = stream.clone();
                    let event_type = event_type.clone();
                    let capability = capability.clone();
                    let producer = producer.clone();
                    let subject_entity = subject_entity.clone();
                    let subject_id = subject_id.clone();
                    let data = data.clone();
                    async move {
                        write_schema_version(&trx, &namespace);
                        let seq_key = evseq_key(&namespace, &stream);
                        let last = match trx.get(&seq_key, false).await? {
                            Some(slice) => decode_u64(&slice),
                            None => 0,
                        };
                        let sequence = last + 1;
                        let event_id = format!("{}-{}", clean_key(&stream), sequence);
                        let envelope = json!({
                            "event_id": event_id.clone(),
                            "event_type": event_type.clone(),
                            "capability": capability.clone(),
                            "producer": producer.clone(),
                            "tenant": namespace.tenant_id.clone(),
                            "subject": {
                                "type": subject_entity.clone(),
                                "id": subject_id.clone()
                            },
                            "payload": data.clone()
                        });
                        let record = EventRecord {
                            event_id,
                            stream: stream.clone(),
                            event_type: event_type.clone(),
                            subject_entity: subject_entity.clone(),
                            subject_id: subject_id.clone(),
                            data: data.clone(),
                            envelope,
                            sequence,
                        };
                        let ekey = event_key(&namespace, &stream, sequence);
                        let encoded =
                            serde_json::to_vec(&record).map_err(|err| lift(encode_error(err)))?;
                        trx.set(&ekey, &encoded);
                        trx.set(&seq_key, &sequence.to_le_bytes());
                        Ok(record)
                    }
                })
                .await
                .map_err(lower)
        })
    }

    fn query_index(&self, op: IndexQueryOp) -> SorxResult<IndexQueryResult> {
        // Parity with the in-memory provider: index queries reduce to an
        // equality query over the collection.
        let result = self.query(QueryOp {
            namespace: op.namespace,
            entity: op.entity,
            collection: op.collection,
            filter: op.filter,
            order_by: Vec::new(),
        })?;
        Ok(IndexQueryResult {
            records: result.records,
        })
    }

    fn traverse(&self, op: TraverseOp) -> SorxResult<TraverseResult> {
        // Parity stub: the in-memory provider only returns the root record
        // (no relationship walking yet). Match that behavior exactly.
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
        self.block_on(async move {
            let key = subject_key(&op.namespace, "xref", &op.collection, &op.entity, &op.id);
            let refs: Vec<ExternalRef> = self
                .db
                .run(|trx, _| {
                    let key = key.clone();
                    async move {
                        match trx.get(&key, false).await? {
                            Some(slice) => decode_json(&slice).map_err(lift),
                            None => Ok(Vec::new()),
                        }
                    }
                })
                .await
                .map_err(lower)?;
            Ok(ExternalRefsResult { refs })
        })
    }

    fn store_evidence(&self, op: StoreEvidenceOp) -> SorxResult<()> {
        self.block_on(async move {
            let namespace = op.namespace;
            let collection = op.collection;
            let entity = op.entity;
            let id = op.id;
            let evidence = op.evidence;

            self.db
                .run(|trx, _| {
                    let namespace = namespace.clone();
                    let collection = collection.clone();
                    let entity = entity.clone();
                    let id = id.clone();
                    let evidence = evidence.clone();
                    async move {
                        let key = subject_key(&namespace, "evid", &collection, &entity, &id);
                        let mut existing: Vec<Value> = match trx.get(&key, false).await? {
                            Some(slice) => decode_json(&slice).map_err(lift)?,
                            None => Vec::new(),
                        };
                        existing.push(evidence);
                        let encoded =
                            serde_json::to_vec(&existing).map_err(|err| lift(encode_error(err)))?;
                        trx.set(&key, &encoded);
                        Ok(())
                    }
                })
                .await
                .map_err(lower)
        })
    }

    fn get_evidence(&self, op: ExternalRefsOp) -> SorxResult<EvidenceResult> {
        self.block_on(async move {
            let key = subject_key(&op.namespace, "evid", &op.collection, &op.entity, &op.id);
            let evidence: Vec<Value> = self
                .db
                .run(|trx, _| {
                    let key = key.clone();
                    async move {
                        match trx.get(&key, false).await? {
                            Some(slice) => decode_json(&slice).map_err(lift),
                            None => Ok(Vec::new()),
                        }
                    }
                })
                .await
                .map_err(lower)?;
            Ok(EvidenceResult { evidence })
        })
    }
}

// ---------------------------------------------------------------------------
// Transaction helpers (async, run inside `Database::run`)
// ---------------------------------------------------------------------------

fn write_schema_version(trx: &foundationdb::RetryableTransaction, namespace: &ProviderNamespace) {
    // v1 only; reserve the key as a migration hook.
    trx.set(&schema_key(namespace), b"1");
}

async fn count_entities(
    trx: &foundationdb::RetryableTransaction,
    namespace: &ProviderNamespace,
    collection: &str,
) -> Result<usize, FdbBindingError> {
    Ok(scan_entities(trx, namespace, collection).await?.len())
}

async fn scan_entities(
    trx: &foundationdb::RetryableTransaction,
    namespace: &ProviderNamespace,
    collection: &str,
) -> Result<Vec<EntityRecord>, FdbBindingError> {
    let mut prefix = entity_prefix(namespace, collection);
    // Append the trailing separator so we only match this collection's keys,
    // not collections sharing a name prefix.
    prefix.push(b'/');
    let end = prefix_end(&prefix);
    let range = RangeOption::from((
        KeySelector::first_greater_or_equal(prefix.clone()),
        KeySelector::first_greater_or_equal(end),
    ));
    let values = trx.get_range(&range, 1, false).await?;
    let mut out = Vec::with_capacity(values.len());
    for kv in &values {
        let record = decode_record(kv.value()).map_err(lift)?;
        out.push(record);
    }
    Ok(out)
}

struct UniqueConflict {
    index: UniqueIndex,
    values: Vec<Value>,
    record: EntityRecord,
}

async fn find_unique_conflict(
    trx: &foundationdb::RetryableTransaction,
    namespace: &ProviderNamespace,
    collection: &str,
    indexes: &[UniqueIndex],
    data: &Value,
    current_id: Option<&str>,
) -> Result<Option<UniqueConflict>, FdbBindingError> {
    for index in indexes {
        let Some(values) = index_values(data, &index.fields) else {
            continue;
        };
        let ukey = uniq_key(namespace, collection, &index.id, &values);
        if let Some(slice) = trx.get(&ukey, false).await? {
            let existing_id = String::from_utf8_lossy(&slice).to_string();
            if current_id == Some(existing_id.as_str()) {
                continue;
            }
            // Load the existing record (for ReturnExisting behavior).
            let ekey = entity_key(namespace, collection, &existing_id);
            if let Some(record_slice) = trx.get(&ekey, false).await? {
                let record = decode_record(&record_slice).map_err(lift)?;
                return Ok(Some(UniqueConflict {
                    index: index.clone(),
                    values,
                    record,
                }));
            }
        }
    }
    Ok(None)
}

async fn clear_unique_for_record(
    trx: &foundationdb::RetryableTransaction,
    namespace: &ProviderNamespace,
    collection: &str,
    record: &EntityRecord,
) -> Result<(), FdbBindingError> {
    // Scan all unique-index keys for this collection and clear the ones that
    // point at this record's id. This keeps delete correct regardless of which
    // indexes the caller passed (DeleteOp carries none).
    let mut prefix = join(&ns_subspace(namespace), &["uniq", &clean_key(collection)]);
    prefix.push(b'/');
    let end = prefix_end(&prefix);
    let range = RangeOption::from((
        KeySelector::first_greater_or_equal(prefix),
        KeySelector::first_greater_or_equal(end),
    ));
    let values = trx.get_range(&range, 1, false).await?;
    for kv in &values {
        if kv.value() == record.id.as_bytes() {
            trx.clear(kv.key());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Encoding / decoding
// ---------------------------------------------------------------------------

fn encode_record(record: &EntityRecord) -> Result<Vec<u8>, SorxError> {
    serde_json::to_vec(record).map_err(encode_error)
}

fn decode_record(bytes: &[u8]) -> Result<EntityRecord, SorxError> {
    serde_json::from_slice(bytes).map_err(decode_error)
}

fn decode_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, SorxError> {
    serde_json::from_slice(bytes).map_err(decode_error)
}

fn decode_u64(bytes: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    let len = bytes.len().min(8);
    buf[..len].copy_from_slice(&bytes[..len]);
    u64::from_le_bytes(buf)
}

fn encode_error(err: serde_json::Error) -> SorxError {
    SorxError::new("provider_encode_failed", err.to_string())
}

fn decode_error(err: serde_json::Error) -> SorxError {
    SorxError::new("provider_decode_failed", err.to_string())
}

// ---------------------------------------------------------------------------
// Shared semantics (parity with the in-memory provider)
// ---------------------------------------------------------------------------

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
        (Some(left), Some(right)) => value_sort_key(left).cmp(&value_sort_key(right)),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

fn value_sort_key(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
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

fn clean_key(value: &str) -> String {
    value
        .trim_matches('/')
        .replace(['/', '\\'], "_")
        .chars()
        .filter(|ch| !ch.is_control())
        .collect()
}
