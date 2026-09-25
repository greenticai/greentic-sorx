# Startup Answers

`greentic-sorx start <pack.gtpack> --schema` emits the startup schema embedded
in the pack. Answer files may use the raw SORX answer object or a
`greentic-qa`-style envelope:

```json
{
  "form_id": "greentic.sorx.start",
  "spec_version": "0.1.0",
  "answers": {
    "tenant": { "tenant_id": "tenant-a" }
  }
}
```

SORX applies schema defaults and validates required values. Use:

```bash
greentic-sorx start landlord.gtpack --answers landlord.answers.json --emit-answers
greentic-sorx start landlord.gtpack --answers landlord.answers.json --dry-run --json
```

Security rules:

- Inline secret-like values are rejected unless they are references.
- Direct provider `config` is allowed only in `local` or `test`.
- Use `config_ref` for shared, staging, and production environments.

Provider entries may include optional ontology/evidence capability metadata:

- `capabilities`: provider capabilities such as `ontology-scoped-evidence-query`
  or `entity-link`.
- `contract_version`: provider contract version, currently compatible with
  `greentic.sorx.provider.v1` or `1`.

Dry-run startup plans use these fields to report provider compatibility for
ontology-enabled packs.

## OCI packs

`start` (alias `run`) accepts an `oci://` reference in place of a local
`.gtpack` path, so a container needs no pack volume — the pack is pulled at
boot:

```bash
greentic-sorx start oci://registry.example/greentic/sor-landlord:1.0.0@sha256:<digest> \
  --answers env:SORX_ANSWERS
```

- **Credentials** are `OCI_USERNAME` / `OCI_PASSWORD` — the same pair
  greentic-start honours, so one Kubernetes Secret serves both. With no
  credentials set, the pull is anonymous HTTPS.
- **Plain HTTP** is used only for hosts listed in
  `GREENTIC_OCI_INSECURE_REGISTRIES` (comma-separated `host[:port]`), and only
  when the reference itself pins a digest (`…@sha256:<hex>`) — an unpinned,
  tag-only reference against a plain-HTTP registry has no transport integrity
  and is refused before any network call. A digest-pinned reference is
  verified against the pulled bytes regardless of transport, so a registry
  that serves something else fails the pull rather than booting a different
  pack.
- **The pushed layer's media type** is `application/vnd.greentic.gtpack.v1+zip`.
- Explicit credentials always win: if `OCI_USERNAME`/`OCI_PASSWORD` are set,
  the pull stays HTTPS even when `GREENTIC_OCI_INSECURE_REGISTRIES` also
  names the host, so a credential is never sent over plain HTTP.

## `--answers env:NAME`

`--answers` may name an environment variable instead of a file:
`--answers env:SORX_ANSWERS` reads the answers JSON from `$SORX_ANSWERS`. The
value is staged into a uniquely named, owner-only temp file (directory
`0700`, file `0600`) for the existing file-based answers loader, and that
file — and its directory — is removed as soon as it has been read. Use it so
a container carries no answers file on disk; an unset or blank variable is
refused by name.

## Postgres store

`providers.store.kind: postgres` keeps records in a Postgres database, used as
an ordered key-value table (`sorx_kv`, created on first connect). It survives
restarts and is safe with several sorx replicas on one database: every
operation runs in one `SERIALIZABLE` transaction, retried on conflict.

The connection string is never an answer. It comes from the environment:

```json
{ "providers": { "store": { "kind": "postgres", "config_ref": "providers.postgres.prod" } } }
```

```bash
SORX_POSTGRES_URL='postgres://user:pass@host:5432/db?sslmode=require' \
  greentic-sorx start landlord.gtpack --answers answers.json
```

Settings, and where each may come from:

| setting | environment variable (any environment) | `config` key (`local` / `test` only) |
|---|---|---|
| connection string | `SORX_POSTGRES_URL` | `url_env` (another variable), `url_file` (a mounted secret) |
| extra trusted CA (PEM) | `SORX_POSTGRES_CA_FILE` | `ca_file` |
| pool size (default 8) | — | `pool_size` |

Direct `config` is refused outside `local` and `test`, so a production
deployment sets the two environment variables. Unknown or wrongly typed
`config` keys are refused at start-up rather than ignored.

**TLS.** `sslmode` in the URL decides whether TLS is used. Unlike libpq,
`sslmode=require` here VERIFIES the server certificate (against the public
webpki roots plus `SORX_POSTGRES_CA_FILE`), and `verify-ca` / `verify-full`
are not accepted. Managed databases (RDS/Aurora, Cloud SQL, Supabase) sign
with their own CAs, so point `SORX_POSTGRES_CA_FILE` at the provider's CA
bundle. `sslmode=disable` also works, and sends credentials in plaintext.

**Privileges.** At start-up sorx creates `sorx_kv` only when it is absent, so
a role granted `SELECT, INSERT, UPDATE, DELETE` on a table a DBA created is
enough.

**Behaviour shared with the other stores, worth knowing:** a `create` with an
explicit `id` that already exists replaces that record, and an idempotent
replay returns the record as it was when first created. Neither is specific to
this store.
