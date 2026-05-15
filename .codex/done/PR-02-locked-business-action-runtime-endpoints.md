# PR 02 — Add locked business action runtime endpoints

## Repository

`greenticai/greentic-sorx`

## Depends on

PR 01: SORX can load `assets/sorla/business-actions.json`, validate
`assets/sorla/business-actions.lock.json`, and expose the loaded catalog and
lock through `LoadedSorlaPack`.

## Objective

Expose runtime HTTP endpoints for locked business actions.

Runtime invocation must not use fuzzy intent matching. It must require action ID/version and verify contract hash through loaded lock metadata.

## New HTTP endpoints

Add:

```http
GET  /v1/sorx/business-actions
GET  /v1/sorx/business-actions/{id}
GET  /v1/sorx/business-actions/{id}/versions/{version}
POST /v1/sorx/business-actions/{id}/versions/{version}/dry-run
POST /v1/sorx/business-actions/{id}/versions/{version}/invoke
```

Optional:

```http
GET /v1/sorx/business-actions/{id}/versions/{version}/schema
```

## Design-time endpoints

`GET /v1/sorx/business-actions` returns action summaries for design-time
discovery: id, available versions, labels, aliases, risk, approval mode,
idempotency requirement, and designer metadata.

`GET /v1/sorx/business-actions/{id}` returns all versions for one action.

`GET /v1/sorx/business-actions/{id}/versions/{version}` returns the selected
catalog action including schemas and execution metadata.

These endpoints can support search in the future, but runtime invocation must
remain locked to explicit id/version/hash checks.

## Runtime request shape

The URL supplies `id` and `version`. The request body supplies the expected
contract hash and invocation payload:

```json
{
  "action_ref": {
    "contract_hash": "sha256:..."
  },
  "values": {
    "tenant_id": "tenant_123",
    "unit_id": "unit_7b",
    "amount": 1250,
    "paid_on": "2026-05-14"
  },
  "options": {
    "idempotency_key": "tenant_123-rent-2026-05",
    "require_explanation": true
  }
}
```

The runtime must reject a body that supplies a conflicting id/version if legacy
clients include those fields in `action_ref`.

## Runtime validation

Before invoking the underlying endpoint/tool, SORX must check:

1. action exists
2. version exists for the action
3. contract hash matches lock metadata
4. input payload validates against action input schema
5. no unknown fields unless schema allows them
6. idempotency key present when action is side-effectful and requires one
7. approval/policy gates pass or return approval-required
8. execution target exists
9. provider/runtime binding is available

## Dry run behavior

`dry-run` should return:

```json
{
  "valid": true,
  "canonical_payload": {},
  "policy_decision": "allow",
  "approval_required": false,
  "execution_target": {},
  "explain": {}
}
```

No provider mutation, audit completion event, or idempotency write may occur.
Policy and approval evaluation should still run so callers can see whether an
invoke would be allowed, denied, or require approval.

## Invoke behavior

`invoke` should execute only after all checks pass.

Return:

```json
{
  "ok": true,
  "action_ref": {},
  "result": {},
  "audit": {
    "event_id": "..."
  },
  "explain": {}
}
```

For approval-required actions, `invoke` must return the same stable
approval-required response without executing the underlying provider operation.

## Failure behavior

Use stable error codes:

```text
unknown_action
unknown_action_version
version_mismatch
contract_hash_mismatch
invalid_payload
missing_idempotency_key
policy_denied
approval_required
execution_target_missing
provider_unavailable
```

## Tests

Add tests for:

- list business action summaries
- get action by ID across versions
- get action by ID/version
- dry-run valid action
- invoke valid action
- reject unknown action
- reject unknown version
- reject version mismatch
- reject hash mismatch
- reject invalid payload
- reject missing idempotency key
- approval-required result
- dry-run does not mutate provider state
- invoke emits audit event
- existing generated route invocation still works

## Docs

Update:

- `docs/business-actions.md`
- `docs/security.md`
- `docs/observability.md`

## Acceptance criteria

```bash
cargo test --all-features
bash ci/local_check.sh
```
