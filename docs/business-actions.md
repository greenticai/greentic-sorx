# Business Actions

SORX can load optional locked business action catalogs from SoRLa `.gtpack`
artifacts.

```text
assets/sorla/business-actions.json
assets/sorla/business-actions.lock.json
```

Packs without a business action catalog remain valid. When
`business-actions.json` is present, `business-actions.lock.json` is required so
runtime calls can use explicit action references instead of inferred intent.

## Catalog

Catalogs use schema `greentic.sorla.business-actions.v1` and contain an
`actions` array. Each action declares:

- `id`
- `version`
- `execution.endpoint_id`, `execution.operation_id`, or `execution.tool_name`
- optional `input_schema` and `output_schema`
- optional `input_bindings`
- optional `risk`, `approval`, and `idempotency`
- optional label, aliases, description, designer metadata, and metadata

Labels, aliases, descriptions, and designer metadata are for discovery. They are
not part of the locked runtime contract hash.

## Lock

Locks use schema `greentic.sorla.business-actions.lock.v1`:

```json
{
  "schema": "greentic.sorla.business-actions.lock.v1",
  "entries": [
    {
      "id": "record_rent_payment",
      "version": "0.1.0",
      "contract_hash": "sha256:..."
    }
  ]
}
```

SORX recomputes each hash from the action id, version, execution target, input
schema, output schema, input bindings, risk, approval, and idempotency
requirements. A mismatch fails `doctor`.

## Manifest Discovery

SORX prefers explicit paths in `pack.cbor` extension metadata:

```json
{
  "sorla": {
    "business_actions": "assets/sorla/business-actions.json",
    "business_actions_lock": "assets/sorla/business-actions.lock.json"
  }
}
```

When metadata is absent, SORX falls back to the standard paths listed above.

## Doctor Checks

`greentic-sorx doctor <pack.gtpack> --json` validates:

- supported catalog and lock schemas
- unique action id/version pairs
- each action has a lock entry
- each lock entry references an action
- recomputed contract hashes match the lock
- execution targets reference generated endpoints, operations, or MCP tools
- input and output schemas are JSON objects when present
- no secret-like values appear in catalog or lock data

## Inspect Summary

`greentic-sorx inspect <pack.gtpack> --json` includes:

```json
{
  "business_actions": {
    "present": true,
    "count": 1,
    "lock_present": true,
    "hashes_valid": true,
    "execution_targets_valid": true
  }
}
```

## Runtime Endpoints

When a started pack contains a valid catalog, the local HTTP runtime exposes:

```http
GET  /v1/sorx/business-actions
GET  /v1/sorx/business-actions/{id}
GET  /v1/sorx/business-actions/{id}/versions/{version}
GET  /v1/sorx/business-actions/{id}/versions/{version}/schema
POST /v1/sorx/business-actions/{id}/versions/{version}/dry-run
POST /v1/sorx/business-actions/{id}/versions/{version}/invoke
```

Design-time endpoints return labels, aliases, schemas, risk, approval mode,
idempotency requirements, and designer metadata. Runtime `dry-run` and `invoke`
use only the explicit URL id/version and request contract hash.

```json
{
  "action_ref": {
    "contract_hash": "sha256:..."
  },
  "values": {
    "tenant_id": "tenant_123"
  },
  "options": {
    "idempotency_key": "tenant_123-rent-2026-05"
  }
}
```

`dry-run` validates the contract hash, payload, policy, provider binding, and
execution target without mutating provider state. `invoke` runs through the same
runtime path as generated HTTP routes after those checks pass.

Stable runtime error codes include:

```text
unknown_action
unknown_action_version
version_mismatch
missing_contract_hash
invalid_contract_hash
contract_hash_mismatch
invalid_payload
missing_idempotency_key
policy_denied
approval_required
execution_target_missing
provider_unavailable
```

Stable doctor codes for catalog and lock drift include:

```text
business_action_lock_missing
business_action_lock_unknown_action
business_action_contract_hash_mismatch
business_action_execution_target_missing
business_action_schema_invalid
secret_like_value
```
