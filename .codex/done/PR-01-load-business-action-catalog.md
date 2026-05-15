# PR 01 — Load and validate business action catalogs in SORX

## Repository

`greenticai/greentic-sorx`

## Objective

Teach SORX to load and validate:

```text
assets/sorla/business-actions.json
assets/sorla/business-actions.lock.json
```

from SoRLa `.gtpack` artifacts.

This PR owns only catalog loading, lock validation, doctor checks, inspect output,
and route/tool correlation metadata. Runtime HTTP invocation belongs to PR 02.

Packs without a business action catalog remain valid. If a catalog is present, the
business action lock is required because PR 02 relies on locked action references.

## New models

Add typed models:

- `BusinessActionCatalog`
- `BusinessAction`
- `BusinessActionRef`
- `BusinessActionExecution`
- `BusinessActionLock`
- `BusinessActionInputBinding`
- `BusinessActionRisk`
- `BusinessActionApproval`
- `BusinessActionContract`

## Pack loader

Update `greentic-sorx-pack` to discover the catalog through pack manifest
extension metadata when available, with fallback to the standard paths above for
compatibility.

Recognize extension metadata under `manifest.extension.sorla`, following the
existing loader pattern:

```json
{
  "business_actions": "assets/sorla/business-actions.json",
  "business_actions_lock": "assets/sorla/business-actions.lock.json"
}
```

Add the loaded catalog and lock to `LoadedSorlaPack::sorla_assets`.

## Contract hashes

Define one deterministic contract hash format for both the catalog lock and PR
02 runtime checks:

- algorithm: SHA-256
- format: `sha256:<hex>`
- canonical input: action id, version, execution target, input schema, output
  schema, input bindings, risk, approval, and idempotency requirements
- excluded from the hash: labels, descriptions, aliases, and designer-only
  metadata

## Doctor validation

`greentic-sorx doctor <pack.gtpack>` should check:

1. catalog schema supported
2. action IDs unique
3. action id/version pairs unique
4. each action has a lock entry
5. each lock entry references an action id/version pair
6. contract hashes recompute correctly
7. execution targets reference generated agent endpoints or MCP tools
8. input schemas are valid JSON Schema objects
9. output schemas are valid JSON Schema objects
10. no secret-like values appear in catalog or lock data
11. risk, approval, and idempotency modes are valid
12. the business action lock is present when the catalog is present

## Inspect output

Add:

```json
{
  "business_actions": {
    "present": true,
    "count": 6,
    "lock_present": true,
    "hashes_valid": true,
    "execution_targets_valid": true
  }
}
```

## CLI route metadata

Ensure existing:

```bash
greentic-sorx routes pack.gtpack --json
greentic-sorx mcp-tools pack.gtpack
```

can be correlated with business action execution targets by including the
referenced `endpoint_id`, `operation_id`, and/or MCP tool name in the catalog
model and validation output. Do not add runtime invocation endpoints in this PR.

## Tests

Add tests for:

- valid catalog load
- pack without catalog remains valid
- catalog without lock fails doctor
- hash mismatch fails doctor
- lock entry without matching action fails doctor
- invalid endpoint reference fails doctor
- invalid MCP tool reference fails doctor
- inspect output stable
- manifest extension paths take precedence over fallback paths
- secret-like catalog or lock values fail doctor

## Docs

Add:

```text
docs/business-actions.md
```

Update:

- `docs/commands.md`
- `docs/validation-suites.md`

## Acceptance criteria

```bash
cargo test --all-features
cargo run -p greentic-sorx -- doctor <business-action-pack.gtpack> --json
cargo run -p greentic-sorx -- inspect <business-action-pack.gtpack> --json
bash ci/local_check.sh
```
