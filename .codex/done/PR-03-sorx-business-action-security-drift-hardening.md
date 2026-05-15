# PR 03 — Harden SORX locked business action security and drift checks

## Repository

`greenticai/greentic-sorx`

## Depends on

- PR 01: catalog and lock loading/validation
- PR 02: versioned locked business action runtime endpoints

## Objective

Harden the locked business action path for production safety inside SORX.

This PR owns SORX runtime, validation, audit, and documentation changes only.
Do not add SoRLa generator work, component behavior, or Flow authoring changes
to this scope.

## Required policies

### Runtime action selection

Runtime must not use fuzzy intent matching to select actions.

Allowed:

```text
action id from URL + version from URL + contract_hash from request body
```

Not allowed:

```text
phrase -> inferred action -> execute
label or alias -> inferred action -> execute
missing version -> latest version -> execute
```

Design-time list/search may use labels and aliases, but invoke and dry-run must
resolve only through explicit id/version/hash.

### Contract drift

Any mismatch between expected and actual contract hash must fail closed before
policy evaluation or provider invocation.

SORX must return stable drift errors for:

- missing contract hash
- malformed contract hash
- contract hash mismatch
- catalog action missing from lock
- lock entry missing from catalog
- recomputed lock hash mismatch during doctor validation

Use the same canonical hash definition from PR 01.

### Idempotency

Side-effectful actions must require an idempotency key unless the catalog
explicitly marks idempotency as not required.

The runtime should:

- reject missing idempotency keys before provider invocation
- preserve the existing operation-scoped idempotency behavior
- include idempotency presence, not raw idempotency values, in audit output
- keep dry-run free of idempotency writes

### Approval and policy

Approval-gated actions must not mutate provider state until approval is granted
by a supported SORX approval path.

For this PR, if no approval continuation flow exists, return a stable
`approval_required` response and do not execute the provider operation.

Policy denial and approval-required responses must be distinguishable:

```text
policy_denied
approval_required
```

### Audit

Each locked action invoke attempt should emit structured audit data containing:

- action id
- action version
- expected contract hash
- actual contract hash or validation failure
- validation result
- policy decision
- approval decision
- idempotency key present
- execution target
- result status

Do not include sensitive payload values unless an existing SORX audit redaction
policy explicitly allows them.

### Secrets

Doctor validation should reject or warn, matching existing severity conventions,
for secret-like values in:

- `assets/sorla/business-actions.json`
- `assets/sorla/business-actions.lock.json`
- runtime request explain/audit fields
- docs or fixtures added by this PR

## Stable error codes

Runtime responses should use stable codes:

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

Doctor responses should use stable codes for catalog and lock drift rather than
collapsing every failure into a generic validation error.

## Tests

Add negative and audit-focused tests for:

- missing contract hash
- malformed contract hash
- contract hash mismatch
- catalog/lock drift
- unknown action
- unknown action version
- conflicting id/version in legacy request body
- missing idempotency key
- invalid payload
- policy denied
- approval required
- provider unavailable
- secret-like values in catalog or lock
- dry-run emits no provider mutation and no idempotency write
- audit omits sensitive payload values

## Docs

Update:

- `docs/business-actions.md`
- `docs/security.md`
- `docs/observability.md`
- `docs/validation-suites.md`

## Acceptance criteria

All locked business action negative cases are tested in this repo, stable error
codes are documented, and SORX fails closed before provider invocation whenever
contract, schema, policy, approval, or idempotency checks fail.
