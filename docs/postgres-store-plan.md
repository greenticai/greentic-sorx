# Postgres canonical store and container image (SoRLa storage, phase 2)

Phase 2 of greentic-designer's
`docs/superpowers/specs/2026-09-24-sorla-storage-for-flows-and-workers-design.md`.
Decision D3 there: the durable store for records sorx masters is a
customer-provided Postgres database, used as an ordered key-value store.
Phase 3 (the deployer shape and the env-canvas SoR unit) needs two things from
this repository, and this plan delivers both:

1. a `postgres` store kind that survives restarts and is safe with more than
   one replica, and
2. an official container image that the k8s and Cloud Run env-packs can run.

## 1. Store

### Shape

One table per database, created on first connect:

```sql
CREATE TABLE IF NOT EXISTS sorx_kv (
    key   BYTEA PRIMARY KEY,
    value BYTEA NOT NULL
);
```

The keys are byte-for-byte the keys `FoundationDbStore` builds
(`sorx/{tenant}/{sor}/e/{collection}/{id}`, `…/idem/…`, `…/uniq/…`,
`…/ev/{stream}/{seq:020}`, `…/evseq/{stream}`, `…/xref/…`, `…/evid/…`,
`…/meta/schema_version`, `…/migrations/{id}`). A prefix scan is
`key >= $start AND key < $end ORDER BY key`, where `$end` is the same
`prefix_end` (strinc) FoundationDB uses.

`value` is `BYTEA`, not the `JSONB` the design sketched. Two sibling
keyspaces are not JSON: the event counter is a little-endian `u64`, and a
unique-index entry is the bare entity id. Storing bytes keeps the two stores
interchangeable and keeps migration and export simple.

### Transactions

Every operation that touches more than one key runs in one transaction at
`SERIALIZABLE` isolation. It retries on SQLSTATE `40001`
(serialization_failure) and `40P01` (deadlock_detected), up to 8 attempts
with jittered backoff. This is the same contract `Database::run` gives the
FoundationDB store. Unique constraints and idempotency therefore hold with
several sorx replicas writing to one database. A `SorxError` raised inside the
transaction body (for example a unique conflict) aborts it and is returned as
is, never retried.

### Code layout

Three deliberate differences from the FoundationDB store, all in the shared
`kv` module. First, an update releases the old value of each unique index it
NAMES whose value changed (and only while the entry is still this record's);
FoundationDB leaves it, so a value a record moved away from stays taken. An
update naming no indexes touches no index entry. Third, auto-ids come from a
per-collection counter (`idseq/{collection}`, seeded from the record count),
not from the count itself: after a delete the count drops and a count-derived
id lands on a live record, which the write then overwrites. Second, the `meta/schema_version` marker is written only
when absent; an unconditional write makes every writer in a namespace update
the same row, which serializes them on its lock and quietly becomes a second,
unintended guarantee behind the unique index. The multi-writer test was
verified to FAIL at `READ COMMITTED` once that was removed, so the guarantee
it checks is the isolation level's.

The operation logic (create, get, update, query, delete, events, index
queries, traversal, external refs, evidence, migration ledger) is written once
against a small key-value transaction trait (`get`, `set`, `clear`,
`scan_prefix`). `PostgresStore` implements that trait over a
`postgres::Transaction`. The FoundationDB store is left untouched: this change
adds a backend, it does not refactor a working one.

### Client and runtime

This uses the synchronous `postgres` crate with an `r2d2` pool (default size 8,
configurable). The HTTP runtime is a synchronous `TcpListener` loop, but the
NATS event bridge calls the store from a multi-thread tokio runtime. Every
database call therefore runs through the same guard `FoundationDbStore::block_on`
uses:

- `block_in_place` when a multi-thread runtime is current;
- a structured error from a current-thread runtime;
- a plain call otherwise.

TLS is rustls with the webpki roots, via `tokio-postgres-rustls`. The URL's own
`sslmode` decides whether it is used.

### Configuration

```json
{ "providers": { "store": { "kind": "postgres",
  "config": { "url_env": "SORX_POSTGRES_URL", "pool_size": 8 } } } }
```

The connection string is never written into the answers:

- `url_env` names an environment variable, default `SORX_POSTGRES_URL`;
- `url_file` names a file, for a mounted secret.

Exactly one of them must resolve. Direct `config` is only accepted in the
`local` and `test` environments (sorx's own rule), so production uses the
environment variables `SORX_POSTGRES_URL` and `SORX_POSTGRES_CA_FILE`. A missing, empty, or unreachable URL fails
start-up with `provider_postgres_error` naming the variable or file, never the
value.

The QA flow offers `postgres` beside `memory` and `foundationdb`.

### Feature

The code sits behind a `postgres` cargo feature on `greentic-sorx-core`, which
`greentic-sorx-cli` enables by default so every released binary and the image
carry it. It needs no system library, unlike `foundationdb`.

### Tests

- **Unit tests:** key construction and the strinc range end.
- **Integration tests** in `crates/greentic-sorx-core/tests/postgres_store.rs`,
  gated on `SORX_TEST_POSTGRES_URL` (skipped when unset). They cover:
  - the full `SorStoreProvider` + `SorxCanonicalStore` surface;
  - a restart (drop the store, reconnect, read back);
  - concurrent creates racing one unique index from several threads, where
    exactly one wins;
  - the migration ledger.
- **CI:** a `postgres:16` service container sets `SORX_TEST_POSTGRES_URL`, so
  the gate actually runs.

## 2. Container image

`ghcr.io/greenticai/greentic-sorx`:

- a multi-arch (linux/amd64 + linux/arm64) image built on push to `develop`;
- tagged with the crate version and `develop`;
- based on distroless cc;
- contains the `greentic-sorx` binary and nothing else;
- entrypoint `greentic-sorx`, non-root.

## 3. Out of scope

- The deployer shape, the env-canvas SoR unit, and staging (phase 3).
- A persistent `memory` kind.
- Deployer-provisioned databases.
