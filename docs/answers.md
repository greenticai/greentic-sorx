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

In `local` or `test` environments, `config` may name a different variable
(`url_env`), a file holding the URL (`url_file`, for a mounted secret), and a
`pool_size` (default 8). TLS is used whenever the URL's `sslmode` asks for it,
verified against the webpki root set.
