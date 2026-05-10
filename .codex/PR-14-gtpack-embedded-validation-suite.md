# PR 14 — Pack-Embedded Automatic Validation Suite

## Goal

Define and execute a validation suite embedded in each SORLA `.gtpack` so SORX can decide whether a deployment is safe to activate and later make public.

This PR creates the validation mechanism. PR 15 uses the result as the public endpoint gate.

## Pack contract

Support optional validation assets inside the `.gtpack`:

```text
assets/sorx/validation-suite.cbor
assets/sorx/validation-suite.json
assets/sorx/validation-fixtures/**
assets/sorx/validation-expected/**
```

The JSON form is for inspection/debugging. The CBOR form is the canonical form when present.

## Validation suite schema

Define schema `greentic.sorx.validation-suite.v1`.

Minimum shape:

```json
{
  "schema": "greentic.sorx.validation-suite.v1",
  "suite_id": "landlord-tenant-basic-public-readiness",
  "pack_name": "landlord-tenant-sor",
  "pack_version": "1.1.0",
  "requires": {
    "provider_kinds": ["foundationdb"],
    "state_mode": "isolated_or_empty_shared",
    "network": "none"
  },
  "gates": {
    "required_for_private_activation": true,
    "required_for_public_exposure": true,
    "minimum_pass_level": "required"
  },
  "tests": [
    {
      "id": "doctor.pack.valid",
      "kind": "doctor",
      "level": "required"
    },
    {
      "id": "openapi.overlay.valid",
      "kind": "artifact_schema",
      "path": "assets/sorla/agent-endpoints.openapi.overlay.yaml",
      "level": "required"
    },
    {
      "id": "tenant.create.happy_path",
      "kind": "endpoint_call",
      "method": "POST",
      "path": "/v1/agent/tenants/create",
      "input_fixture": "assets/sorx/validation-fixtures/tenant-create.json",
      "expect": {
        "status": 200,
        "json_path": "$.ok",
        "equals": true
      },
      "level": "required"
    }
  ]
}
```

## Test kinds

Implement these first:

```text
doctor
artifact_exists
artifact_schema
route_generation
provider_contract
endpoint_call
negative_endpoint_call
audit_event_emitted
idempotency
policy_denial
```

Do not support arbitrary shell execution from packs. Pack validation must be declarative and sandbox-safe.

## Execution model

Validation runs against an ephemeral SORX runtime instance:

```text
load pack
  -> doctor
  -> bind provider in validation mode
  -> create temporary state namespace
  -> generate route table
  -> run declarative endpoint calls
  -> assert responses/events/audit
  -> tear down temporary state unless configured to preserve on failure
```

Provider bindings must support validation mode. If a real provider such as FoundationDB is unavailable, the suite can declare whether an in-memory provider is acceptable.

## Report format

Emit a signed/stable report shape:

```json
{
  "schema": "greentic.sorx.validation-report.v1",
  "deployment_id": "acme-landlord-v1-1",
  "pack_name": "landlord-tenant-sor",
  "pack_version": "1.1.0",
  "pack_digest": "sha256:...",
  "suite_id": "landlord-tenant-basic-public-readiness",
  "started_at": "2026-05-09T00:00:00Z",
  "finished_at": "2026-05-09T00:00:01Z",
  "result": "pass",
  "public_exposure_allowed": true,
  "tests": [
    {
      "id": "tenant.create.happy_path",
      "result": "pass",
      "duration_ms": 12
    }
  ]
}
```

Use deterministic ordering. Timestamps are allowed in reports but not in pack artifacts.

## CLI

Add:

```bash
greentic-sorx validate landlord-tenant-sor.gtpack --answers sorx.answers.json

greentic-sorx deployments validate <deployment-id>

greentic-sorx validation report <deployment-id>
```

Options:

```bash
--provider-mode in-memory|configured|mock
--preserve-state-on-failure
--json
--junit-out target/sorx-validation.xml
```

## Public-readiness levels

Each test has a level:

```text
required
recommended
informational
```

For public exposure:

- all `required` tests must pass
- skipped required tests fail unless explicitly allowed by policy
- recommended failures do not block public exposure but appear in the report
- informational tests never block

## Changes needed in `greentic-sorla`

If this PR is implemented in SORX before SORLA emits the suite, add fixtures manually. Then open a follow-up issue/PR for `greentic-sorla` to emit:

```text
assets/sorx/validation-suite.cbor
assets/sorx/validation-suite.json
assets/sorx/validation-fixtures/**
```

The suite should be generated from wizard answers, endpoint examples, schemas, policy declarations, and provider contract metadata.

## Tests

Add integration tests for:

- pack without suite is valid for private/local but not public when policy requires suite
- valid suite passes
- invalid suite schema fails doctor/validate
- endpoint happy path test passes
- endpoint negative test catches validation error
- idempotency test catches duplicate create behavior
- audit event emitted test works
- required failure blocks public readiness
- recommended failure does not block public readiness
- report is stable and includes pack/deployment identity

## Acceptance criteria

- SORX can discover and execute pack-embedded validation suites.
- Validation reports are tied to deployment ID, pack version, and digest.
- The suite is declarative and does not run arbitrary code from packs.
- Public-readiness decision is available to PR 15.

## Codex working style

Prefer a small, strict schema and a few well-tested test kinds. Do not create a general-purpose test runner that can execute arbitrary code from a pack.
