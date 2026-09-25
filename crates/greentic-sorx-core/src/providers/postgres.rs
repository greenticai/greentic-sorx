//! Postgres-backed canonical store (`--features postgres`).
//!
//! Postgres is used as an ordered key-value store: one table,
//! `sorx_kv (key BYTEA PRIMARY KEY, value BYTEA NOT NULL)`, holding exactly the
//! keyspace the FoundationDB store writes (see [`super::kv`]). A prefix scan is
//! `key >= $1 AND key < $2 ORDER BY key`. `value` is `BYTEA` rather than
//! `JSONB` because two sibling keyspaces are not JSON (the little-endian event
//! counter and the bare id a unique-index entry holds).
//!
//! # Transactions
//!
//! Every operation runs in ONE transaction at `SERIALIZABLE` isolation and is
//! retried on serialization failure (`40001`) and deadlock (`40P01`) — the
//! contract `Database::run` gives the FoundationDB store. That is what keeps
//! unique indexes and idempotency correct with several sorx replicas on one
//! database. A `SorxError` raised by the operation itself (a unique conflict,
//! a missing record) aborts the transaction and is returned, never retried.
//!
//! # Blocking
//!
//! The client is the synchronous `postgres` crate, which drives its own
//! runtime. The HTTP runtime is a synchronous loop, but the NATS event bridge
//! calls the store from a multi-thread tokio runtime, so every call goes
//! through [`run_blocking`]: `block_in_place` inside a multi-thread runtime, a
//! structured error inside a current-thread one, a plain call otherwise.
//!
//! # Configuration
//!
//! The connection string never appears in the answers. It is read from the
//! environment variable named by `config.url_env` (default
//! [`DEFAULT_URL_ENV`]) or from the file named by `config.url_file` (a mounted
//! secret). Errors name the variable or file, never the value.

use std::time::{Duration, Instant};

use postgres::error::SqlState;
use postgres::{IsolationLevel, Transaction};
use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_postgres_rustls::MakeRustlsConnect;

use super::kv::{self, KvTxn};
use crate::migration::runner::AppliedMigrations;
use crate::{
    AppendEventOp, CreateOp, DeleteOp, DeleteResult, EntityRecord, EventRecord, EvidenceResult,
    ExternalRefsOp, ExternalRefsResult, GetOp, IndexQueryOp, IndexQueryResult, ProviderNamespace,
    QueryOp, QueryResult, SorStoreProvider, SorxCanonicalStore, SorxError, SorxResult,
    StoreEvidenceOp, TraverseOp, TraverseResult, UpdateOp,
};

/// Environment variable read for the connection string when the answers name
/// none.
pub const DEFAULT_URL_ENV: &str = "SORX_POSTGRES_URL";
/// Environment variable naming a PEM file of extra trusted root certificates.
/// Managed databases (RDS/Aurora, Cloud SQL, Supabase) sign with CAs that are
/// not in the public webpki set, so without this their TLS never verifies.
pub const CA_FILE_ENV: &str = "SORX_POSTGRES_CA_FILE";
const DEFAULT_POOL_SIZE: u32 = 8;
/// How long one operation may keep retrying serialization conflicts. A time
/// budget, not an attempt count: sixteen writers racing one collection's id
/// counter need up to sixteen rounds each, which a small fixed count cut short.
const RETRY_BUDGET: Duration = Duration::from_secs(10);
const CONFIG_KEYS: &[&str] = &["url_env", "url_file", "ca_file", "pool_size"];
/// `CREATE TABLE IF NOT EXISTS` is not safe against itself: two sessions
/// racing it can both pass the existence check and one then fails on the
/// catalog's unique index. Several sorx replicas booting at once do exactly
/// that, so creation runs under a transaction-scoped advisory lock.
///
/// It runs only when the table is absent: Postgres checks CREATE privilege on
/// the schema BEFORE honouring `IF NOT EXISTS`, so a role granted only DML on a
/// table a DBA created would otherwise be refused at every boot.
const SCHEMA_SQL: &str = "BEGIN; \
    SELECT pg_advisory_xact_lock(7305001); \
    CREATE TABLE IF NOT EXISTS sorx_kv (key BYTEA PRIMARY KEY, value BYTEA NOT NULL); \
    COMMIT;";

type Manager = PostgresConnectionManager<MakeRustlsConnect>;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PostgresProviderConfig {
    /// Environment variable holding the connection string.
    pub url_env: Option<String>,
    /// File holding the connection string (a mounted secret).
    pub url_file: Option<String>,
    /// PEM file of extra trusted roots; [`CA_FILE_ENV`] is read when unset.
    pub ca_file: Option<String>,
    pub pool_size: Option<u32>,
    pub config_ref: Option<String>,
}

impl PostgresProviderConfig {
    /// Read the binding's `config`. Unknown keys and wrong types are REFUSED,
    /// not ignored: a misspelled `url_env` would otherwise fall back to
    /// [`DEFAULT_URL_ENV`], which may name a different database.
    pub fn from_parts(config_ref: Option<String>, config: Option<Value>) -> SorxResult<Self> {
        let object = match config {
            None | Some(Value::Null) => serde_json::Map::new(),
            Some(Value::Object(object)) => object,
            Some(other) => {
                // Name the type only: a value here may well be a pasted
                // connection string, and errors reach logs.
                let kind = match other {
                    Value::String(_) => "a string",
                    Value::Array(_) => "an array",
                    Value::Number(_) => "a number",
                    _ => "a boolean",
                };
                return Err(pg_error(format!(
                    "the postgres store's `config` must be an object, not {kind}"
                )));
            }
        };
        if let Some(unknown) = object
            .keys()
            .find(|key| !CONFIG_KEYS.contains(&key.as_str()))
        {
            return Err(pg_error(format!(
                "the postgres store does not know the config key `{unknown}` (known: {})",
                CONFIG_KEYS.join(", ")
            )));
        }
        let text = |key: &str| -> SorxResult<Option<String>> {
            match object.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(value)) if !value.trim().is_empty() => {
                    Ok(Some(value.trim().to_string()))
                }
                Some(_) => Err(pg_error(format!(
                    "the postgres store's `{key}` must be a non-empty string"
                ))),
            }
        };
        let pool_size = match object.get("pool_size") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|size| u32::try_from(size).ok())
                    .filter(|size| *size > 0)
                    .ok_or_else(|| {
                        pg_error("the postgres store's `pool_size` must be a positive integer")
                    })?,
            ),
        };
        Ok(Self {
            url_env: text("url_env")?,
            url_file: text("url_file")?,
            ca_file: text("ca_file")?,
            pool_size,
            config_ref,
        })
    }

    /// Resolve the connection string. `url_file` wins when both are named.
    pub fn resolve_url(&self) -> SorxResult<String> {
        if let Some(path) = &self.url_file {
            let raw = std::fs::read_to_string(path).map_err(|err| {
                pg_error(format!("cannot read the Postgres URL file `{path}`: {err}"))
            })?;
            let url = raw.trim();
            if url.is_empty() {
                return Err(pg_error(format!("the Postgres URL file `{path}` is empty")));
            }
            return Ok(url.to_string());
        }
        let var = self.url_env.as_deref().unwrap_or(DEFAULT_URL_ENV);
        match std::env::var(var) {
            Ok(url) if !url.trim().is_empty() => Ok(url.trim().to_string()),
            Err(std::env::VarError::NotUnicode(_)) => Err(pg_error(format!(
                "the environment variable `{var}` is set but is not valid Unicode"
            ))),
            _ => Err(pg_error(format!(
                "the postgres store needs a connection string in the environment \
                 variable `{var}` (or name a file with `url_file`)"
            ))),
        }
    }

    /// The extra-roots file: `ca_file`, else [`CA_FILE_ENV`], else none.
    pub fn resolve_ca_file(&self) -> Option<String> {
        self.ca_file.clone().or_else(|| {
            std::env::var(CA_FILE_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
    }
}

/// Postgres-backed canonical store.
pub struct PostgresStore {
    pool: Pool<Manager>,
}

impl std::fmt::Debug for PostgresStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresStore").finish_non_exhaustive()
    }
}

impl PostgresStore {
    /// Resolve the URL, open the pool and create the table when absent.
    pub fn connect(config: &PostgresProviderConfig) -> SorxResult<Self> {
        Self::connect_with(
            &config.resolve_url()?,
            config.pool_size.unwrap_or(DEFAULT_POOL_SIZE),
            config.resolve_ca_file().as_deref(),
        )
    }

    /// Connect with the public webpki roots plus [`CA_FILE_ENV`] when set.
    pub fn connect_url(url: &str, pool_size: u32) -> SorxResult<Self> {
        let ca_file = PostgresProviderConfig::default().resolve_ca_file();
        Self::connect_with(url, pool_size, ca_file.as_deref())
    }

    fn connect_with(url: &str, pool_size: u32, ca_file: Option<&str>) -> SorxResult<Self> {
        let config: postgres::Config = url
            .parse()
            .map_err(|err| pg_error(format!("the Postgres URL does not parse: {err}")))?;
        let tls = tls_connector(ca_file)?;
        let manager = PostgresConnectionManager::new(config.clone(), tls.clone());
        let store = run_blocking(|| {
            // One direct connection first: the pool reports every failure as
            // "timed out waiting for connection" after its timeout, which hides
            // the server's reason (a wrong password, an unknown database).
            config.connect(tls).map_err(|err| {
                pg_error(format!("cannot connect to Postgres: {}", describe(&err)))
            })?;
            let pool = Pool::builder()
                .max_size(pool_size)
                .connection_timeout(Duration::from_secs(10))
                .build(manager)
                .map_err(|err| pg_error(format!("cannot connect to Postgres: {err}")))?;
            let mut client = pool
                .get()
                .map_err(|err| pg_error(format!("cannot connect to Postgres: {err}")))?;
            let exists: bool = client
                .query_one("SELECT to_regclass('sorx_kv') IS NOT NULL", &[])
                .map_err(|err| {
                    pg_error(format!("cannot inspect the database: {}", describe(&err)))
                })?
                .get(0);
            if !exists {
                client.batch_execute(SCHEMA_SQL).map_err(|err| {
                    pg_error(format!(
                        "cannot create the sorx_kv table: {}",
                        describe(&err)
                    ))
                })?;
            }
            Ok(Self { pool })
        })?;
        Ok(store)
    }

    /// Run `body` in one SERIALIZABLE transaction, retrying serialization
    /// failures and deadlocks for up to [`RETRY_BUDGET`].
    fn transact<T>(&self, body: impl Fn(&mut PgTxn<'_, '_>) -> SorxResult<T>) -> SorxResult<T> {
        run_blocking(|| {
            let mut client = self
                .pool
                .get()
                .map_err(|err| pg_error(format!("no Postgres connection available: {err}")))?;
            let started = Instant::now();
            let mut attempt: u32 = 0;
            loop {
                attempt += 1;
                let result = (|| {
                    let mut tx = client
                        .build_transaction()
                        .isolation_level(IsolationLevel::Serializable)
                        .start()
                        .map_err(Failure::from_db)?;
                    let value = {
                        let mut txn = PgTxn { tx: &mut tx };
                        body(&mut txn).map_err(|err| {
                            if err.code == RETRYABLE_CODE {
                                Failure::Retry(err.message)
                            } else {
                                Failure::Sorx(err)
                            }
                        })?
                    };
                    tx.commit().map_err(Failure::from_commit)?;
                    Ok(value)
                })();
                match result {
                    Ok(value) => return Ok(value),
                    Err(Failure::Sorx(err)) => return Err(err),
                    Err(Failure::Retry(last)) => {
                        if started.elapsed() >= RETRY_BUDGET {
                            return Err(pg_error(format!(
                                "Postgres transaction kept conflicting with concurrent \
                                 writers ({attempt} attempts over {:?}; last: {last})",
                                started.elapsed()
                            )));
                        }
                        std::thread::sleep(backoff(attempt));
                    }
                    Err(Failure::Db(message)) => {
                        return Err(pg_error(format!("Postgres transaction failed: {message}")));
                    }
                    Err(Failure::CommitUnknown(message)) => {
                        return Err(SorxError::new(
                            "provider_postgres_commit_unknown",
                            format!(
                                "the connection failed while committing, so the write may or \
                                 may not have been applied; check before retrying: {message}"
                            ),
                        ));
                    }
                }
            }
        })
    }

    pub fn record_migration_applied(
        &self,
        namespace: &ProviderNamespace,
        migration_id: &str,
    ) -> SorxResult<()> {
        self.transact(|txn| kv::record_migration(txn, namespace, migration_id))
    }

    pub fn load_applied_migrations(
        &self,
        namespace: &ProviderNamespace,
    ) -> SorxResult<AppliedMigrations> {
        self.transact(|txn| kv::load_migrations(txn, namespace))
    }
}

/// Why one attempt failed.
enum Failure {
    /// The operation itself refused (conflict, not found, decode): final.
    Sorx(SorxError),
    /// A serialization failure or deadlock: the attempt was NOT applied and
    /// may be retried. Carries the server's message for the give-up error.
    Retry(String),
    /// A definite database failure; nothing was applied.
    Db(String),
    /// COMMIT failed without a server answer (the connection dropped): the
    /// transaction may have been applied. Never retried, never reported as a
    /// plain failure — a caller retrying a create would duplicate it.
    CommitUnknown(String),
}

impl Failure {
    fn from_db(err: postgres::Error) -> Self {
        if is_retryable(&err) {
            Self::Retry(describe(&err))
        } else {
            Self::Db(describe(&err))
        }
    }

    fn from_commit(err: postgres::Error) -> Self {
        if err.as_db_error().is_none() {
            Self::CommitUnknown(describe(&err))
        } else {
            Self::from_db(err)
        }
    }
}

/// Code carried by a `SorxError` that wraps a retryable database failure
/// raised inside the body, so [`PostgresStore::transact`] can tell it from the
/// operation's own refusals.
const RETRYABLE_CODE: &str = "provider_postgres_retryable";

fn is_retryable(err: &postgres::Error) -> bool {
    matches!(
        err.code(),
        Some(code) if *code == SqlState::T_R_SERIALIZATION_FAILURE
            || *code == SqlState::T_R_DEADLOCK_DETECTED
    )
}

/// Jittered backoff: a few milliseconds, growing slowly to a 50 ms ceiling.
/// Jitter comes from the clock and the thread, so writers that collided do not
/// collide again in lockstep.
fn backoff(attempt: u32) -> Duration {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    Instant::now().hash(&mut hasher);
    attempt.hash(&mut hasher);
    let ceiling = u64::from(attempt.min(10)) * 5 + 5;
    Duration::from_millis(1 + hasher.finish() % ceiling)
}

struct PgTxn<'a, 'b> {
    tx: &'a mut Transaction<'b>,
}

impl PgTxn<'_, '_> {
    fn map(err: postgres::Error) -> SorxError {
        if is_retryable(&err) {
            SorxError::new(RETRYABLE_CODE, describe(&err))
        } else {
            pg_error(format!("Postgres query failed: {}", describe(&err)))
        }
    }
}

impl KvTxn for PgTxn<'_, '_> {
    fn get(&mut self, key: &[u8]) -> SorxResult<Option<Vec<u8>>> {
        let row = self
            .tx
            .query_opt("SELECT value FROM sorx_kv WHERE key = $1", &[&key])
            .map_err(Self::map)?;
        Ok(row.map(|row| row.get(0)))
    }

    fn set(&mut self, key: &[u8], value: &[u8]) -> SorxResult<()> {
        self.tx
            .execute(
                "INSERT INTO sorx_kv (key, value) VALUES ($1, $2) \
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
                &[&key, &value],
            )
            .map_err(Self::map)?;
        Ok(())
    }

    fn clear(&mut self, key: &[u8]) -> SorxResult<()> {
        self.tx
            .execute("DELETE FROM sorx_kv WHERE key = $1", &[&key])
            .map_err(Self::map)?;
        Ok(())
    }

    fn scan(&mut self, start: &[u8], end: &[u8]) -> SorxResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let rows = self
            .tx
            .query(
                "SELECT key, value FROM sorx_kv WHERE key >= $1 AND key < $2 ORDER BY key",
                &[&start, &end],
            )
            .map_err(Self::map)?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect())
    }
}

impl crate::migration::MigrationLedger for PostgresStore {
    fn load(&self, namespace: &ProviderNamespace) -> SorxResult<AppliedMigrations> {
        self.load_applied_migrations(namespace)
    }

    fn record_applied(&self, namespace: &ProviderNamespace, migration_id: &str) -> SorxResult<()> {
        self.record_migration_applied(namespace, migration_id)
    }
}

impl SorStoreProvider for PostgresStore {
    fn create(&self, op: CreateOp) -> SorxResult<EntityRecord> {
        self.transact(|txn| kv::create(txn, &op))
    }

    fn get(&self, op: GetOp) -> SorxResult<Option<EntityRecord>> {
        self.transact(|txn| kv::get(txn, &op))
    }

    fn update(&self, op: UpdateOp) -> SorxResult<EntityRecord> {
        self.transact(|txn| kv::update(txn, &op))
    }

    fn query(&self, op: QueryOp) -> SorxResult<QueryResult> {
        let records = self.transact(|txn| kv::query_records(txn, &op))?;
        Ok(QueryResult { records })
    }

    fn delete(&self, op: DeleteOp) -> SorxResult<DeleteResult> {
        self.transact(|txn| kv::delete(txn, &op))
    }
}

impl SorxCanonicalStore for PostgresStore {
    fn append_event(&self, op: AppendEventOp) -> SorxResult<EventRecord> {
        self.transact(|txn| kv::append_event(txn, &op))
    }

    fn query_index(&self, op: IndexQueryOp) -> SorxResult<IndexQueryResult> {
        // Parity with the other providers: an equality query over the collection.
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
        // Parity with the other providers: the root record only.
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
        let refs = self.transact(|txn| {
            kv::get_external_refs(txn, &op.namespace, &op.collection, &op.entity, &op.id)
        })?;
        Ok(ExternalRefsResult { refs })
    }

    fn store_evidence(&self, op: StoreEvidenceOp) -> SorxResult<()> {
        self.transact(|txn| kv::store_evidence(txn, &op))
    }

    fn get_evidence(&self, op: ExternalRefsOp) -> SorxResult<EvidenceResult> {
        let evidence = self.transact(|txn| {
            kv::get_evidence(txn, &op.namespace, &op.collection, &op.entity, &op.id)
        })?;
        Ok(EvidenceResult { evidence })
    }

    fn as_migration_ledger(&self) -> Option<&dyn crate::migration::MigrationLedger> {
        Some(self)
    }
}

/// Run blocking database work without stalling or panicking an async caller.
fn run_blocking<T>(work: impl FnOnce() -> SorxResult<T>) -> SorxResult<T> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread {
                return Err(pg_error(
                    "the postgres store requires a multi-thread Tokio runtime; \
                     it cannot run inside a current-thread runtime",
                ));
            }
            tokio::task::block_in_place(work)
        }
        Err(_) => work(),
    }
}

/// rustls with the public webpki roots, plus the PEM roots in `ca_file`.
///
/// Note the sslmode semantics differ from libpq: `require` VERIFIES the
/// certificate here, and `verify-ca` / `verify-full` are not accepted by the
/// URL parser. A managed database therefore needs its CA in `ca_file` (or
/// [`CA_FILE_ENV`]); `sslmode=disable` is the only way around it, and sends
/// the credentials in plaintext.
fn tls_connector(ca_file: Option<&str>) -> SorxResult<MakeRustlsConnect> {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;

    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    if let Some(path) = ca_file {
        let mut added = 0usize;
        for cert in CertificateDer::pem_file_iter(path)
            .map_err(|err| pg_error(format!("cannot read the Postgres CA file `{path}`: {err}")))?
        {
            let cert = cert.map_err(|err| {
                pg_error(format!(
                    "the Postgres CA file `{path}` is not valid PEM: {err}"
                ))
            })?;
            roots.add(cert).map_err(|err| {
                pg_error(format!(
                    "the Postgres CA file `{path}` holds a bad certificate: {err}"
                ))
            })?;
            added += 1;
        }
        if added == 0 {
            return Err(pg_error(format!(
                "the Postgres CA file `{path}` holds no certificate"
            )));
        }
    }
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| pg_error(format!("cannot configure TLS: {err}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}

/// `postgres::Error`'s `Display` for a server error is just "db error"; the
/// server's own message and SQLSTATE are what an operator can act on.
fn describe(err: &postgres::Error) -> String {
    match err.as_db_error() {
        Some(db) => format!("{} (SQLSTATE {})", db.message(), db.code().code()),
        None => err.to_string(),
    }
}

fn pg_error(message: impl Into<String>) -> SorxError {
    SorxError::new("provider_postgres_error", message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn config_reads_url_env_file_and_pool_size() {
        let config = PostgresProviderConfig::from_parts(
            Some("providers.store".into()),
            Some(json!({"url_env": "MY_PG", "url_file": "/run/pg", "pool_size": 3})),
        )
        .expect("valid config");
        assert_eq!(config.url_env.as_deref(), Some("MY_PG"));
        assert_eq!(config.url_file.as_deref(), Some("/run/pg"));
        assert_eq!(config.pool_size, Some(3));
    }

    #[test]
    fn a_misspelled_or_mistyped_key_is_refused_not_ignored() {
        for bad in [
            json!({"urlEnv": "MY_PG"}),
            json!({"url_env": 5}),
            json!({"url_file": " "}),
            json!({"pool_size": "4"}),
            json!({"pool_size": 0}),
            json!("postgres://secret@host/db"),
        ] {
            let err =
                PostgresProviderConfig::from_parts(None, Some(bad.clone())).expect_err("refused");
            assert_eq!(err.code, "provider_postgres_error", "{bad}");
            assert!(
                !err.message.contains("secret@host"),
                "never echo a value: {}",
                err.message
            );
        }
    }

    #[test]
    fn a_missing_url_names_the_variable_and_never_a_value() {
        let config = PostgresProviderConfig {
            url_env: Some("SORX_TEST_SURELY_UNSET_PG_URL".into()),
            ..Default::default()
        };
        let err = config.resolve_url().expect_err("unset variable");
        assert_eq!(err.code, "provider_postgres_error");
        assert!(err.message.contains("SORX_TEST_SURELY_UNSET_PG_URL"));
    }

    #[test]
    fn url_file_wins_and_is_trimmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("url");
        std::fs::write(&path, "postgres://u@h/db\n").expect("write");
        let config = PostgresProviderConfig {
            url_env: Some("SORX_TEST_SURELY_UNSET_PG_URL".into()),
            url_file: Some(path.display().to_string()),
            ..Default::default()
        };
        assert_eq!(config.resolve_url().expect("url"), "postgres://u@h/db");
    }

    #[test]
    fn a_ca_file_that_is_missing_or_empty_is_refused_by_name() {
        let err = tls_connector(Some("/nonexistent/sorx-ca.pem"))
            .err()
            .expect("missing file");
        assert!(
            err.message.contains("/nonexistent/sorx-ca.pem"),
            "{}",
            err.message
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "").expect("write");
        let err = tls_connector(Some(&empty.display().to_string()))
            .err()
            .expect("empty file");
        assert!(err.message.contains("no certificate"), "{}", err.message);

        tls_connector(None).expect("the public roots alone are fine");
    }
}
