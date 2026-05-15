# PR 04 — Add SORX locked business action e2e coverage

## Repository

`greenticai/greentic-sorx`

## Depends on

- PR 01: SORX loads and validates `assets/sorla/business-actions.json` and
  `assets/sorla/business-actions.lock.json`.
- PR 02: SORX exposes versioned locked business action HTTP endpoints.
- PR 03: SORX fails closed for contract drift, idempotency, policy, approval,
  and provider availability errors.

## Objective

Add deterministic SORX-only coverage for the locked business action path:

```text
.gtpack fixture with business action catalog + lock
  -> greentic-sorx doctor validates catalog and lock
  -> greentic-sorx inspect reports business action metadata
  -> greentic-sorx start serves versioned locked action endpoints
  -> SORX verifies id/version/hash/schema/policy before invoke
  -> result, approval, audit, and idempotency behavior are stable
```

This PR must not depend on SoRLa, Greentic Flow, component test runners, or
other repositories. Build any fixture data needed for the test inside this repo,
following the existing deterministic `.gtpack` fixture helpers.

## Scenario

Use the existing landlord/tenant runtime fixture or a small generic business
fixture. Domain names are fine in fixture data, but SORX contracts should remain
generic.

Example action:

```text
Record monthly rent payment
```

The action reference under test must include:

```json
{
  "id": "record_rent_payment",
  "version": "0.1.0",
  "contract_hash": "sha256:..."
}
```

The runtime call must use the versioned endpoint shape from PR 02:

```http
POST /v1/sorx/business-actions/{id}/versions/{version}/invoke
```

The request body supplies `action_ref.contract_hash`, `values`, and `options`.
Runtime invocation must not rely on text prompts, aliases, labels, or inferred
intent.

## Required assertions

1. Doctor accepts a valid business action catalog and lock.
2. Inspect reports `business_actions.present`, count, lock presence, and valid hashes.
3. Runtime lists business action summaries.
4. Runtime fetches the action by id/version.
5. Dry-run validates payload and policy without mutating provider state.
6. Invoke with the correct hash succeeds.
7. Invoke with the wrong hash fails closed.
8. Invoke with an unknown version fails closed.
9. Side-effectful actions require and honor idempotency.
10. Approval-required actions return approval-required without provider mutation.
11. Audit output includes action id, version, hash result, policy decision, and status.
12. Existing generated route invocation still works alongside business action endpoints.

## Commands

Use repo-local commands only:

```bash
cargo test --all-features
cargo run -p greentic-sorx -- doctor <business-action-pack.gtpack> --json
cargo run -p greentic-sorx -- inspect <business-action-pack.gtpack> --json
cargo run -p greentic-sorx -- start <business-action-pack.gtpack> --answers <answers.json>
bash ci/local_check.sh
```

If the e2e fixture is generated inside a test, document the equivalent manual
command with the generated fixture path only where practical.

## Tests

Add coverage in this repo, preferably near the existing CLI/runtime e2e tests:

- valid locked business action flow
- dry-run does not mutate provider state
- hash mismatch
- unknown action version
- invalid payload
- missing idempotency key
- approval-required response
- audit emitted for invoke
- generated routes still invoke normally

## Docs

Add or update:

- `docs/business-actions.md`
- `docs/e2e-landlord-tenant.md` or a new SORX-local e2e doc
- `docs/commands.md`

## Acceptance criteria

The SORX-only locked business action flow is covered by CI-safe tests and the
documented local commands are accurate for this repository.
