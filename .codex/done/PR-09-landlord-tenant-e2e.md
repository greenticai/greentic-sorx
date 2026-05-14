# PR 09 — Landlord/Tenant End-to-End Scenario

## Goal

Create a realistic e2e test that proves Sorx can run a SoRLa-powered landlord/tenant system of record from a `.gtpack`.

This scenario should validate real product behaviour, not just unit tests.

## Scenario

Entities:

- Landlord
- Property
- Unit
- Tenant
- Tenancy
- Payment
- MaintenanceRequest

Operations:

- create landlord
- create property
- create unit
- create tenant
- assign tenant to unit
- record rent payment
- open maintenance request
- list active tenants
- update tenant contact preferences
- read back all records

## Pack input

Use a real or generated fixture pack:

```text
tests/e2e/fixtures/landlord-tenant.gtpack
```

If `greentic-sorla` pack generation is available in CI, generate it during test setup.

If not, keep a deterministic fixture generated from a known SoRLa example and document how to refresh it.

## Startup answers

Create:

```text
tests/e2e/fixtures/landlord-tenant.answers.memory.json
tests/e2e/fixtures/landlord-tenant.answers.foundationdb.json
```

Memory provider answers should be CI-safe and default.

FoundationDB answers should be optional/manual.

## E2E flow

The test should:

1. Run `greentic-sorx doctor landlord-tenant.gtpack`.
2. Run `greentic-sorx start landlord-tenant.gtpack --answers landlord-tenant.answers.memory.json` on a local port.
3. Call `/healthz` and `/readyz`.
4. List routes.
5. Create landlord.
6. Create property.
7. Create units.
8. Create tenants.
9. Assign active tenancy.
10. Record payment.
11. Open maintenance request.
12. Query active tenants.
13. Update tenant preferred contact method.
14. Re-read tenant and tenancy.
15. Verify audit events were emitted.
16. Verify idempotency on repeated mutating calls.
17. Verify high-risk operation requires approval if included.

## Agent endpoint examples

Example HTTP calls:

```text
POST /v1/agent/landlords/create
POST /v1/agent/properties/create
POST /v1/agent/units/create
POST /v1/agent/tenants/create
POST /v1/agent/tenancies/assign
POST /v1/agent/payments/record
POST /v1/agent/maintenance/open
GET  /v1/agent/landlords/{landlord_id}/active-tenants
```

Use the actual routes from `agent-gateway.json`. Do not hard-code if the pack differs.

## Schema migration extension

If Sorx already supports schema version handling, extend the e2e to:

- start with v1 pack
- create records
- start or doctor v2 pack with added fields
- verify old records still load
- update new fields
- verify migration/idempotency

If schema migration is not yet supported, document as follow-up and keep the e2e focused on runtime execution.

## MCP extension

If MCP runtime from PR 07 exists, add MCP e2e calls for:

- create tenant
- assign tenancy
- list active tenants

Otherwise add TODO/follow-up.

## Test structure

Suggested:

```text
tests/e2e/
  landlord_tenant_memory.rs
  landlord_tenant_foundationdb.rs

tests/e2e/fixtures/landlord_tenant/
  landlord-tenant.gtpack
  answers.memory.json
  answers.foundationdb.json
  expected-routes.json
  seed-data.json
```

## Developer commands

Add:

```bash
cargo xtask e2e landlord-tenant --provider memory
cargo xtask e2e landlord-tenant --provider foundationdb
```

If no `xtask` exists, add script:

```bash
scripts/e2e/run-landlord-tenant.sh --provider memory
```

## Tests

Automated checks:

- e2e memory provider runs in CI
- route list matches expected
- created records can be read back
- query active tenants returns expected tenant
- idempotency prevents duplicate tenant/payment
- high-risk operation requires approval where applicable
- audit sink receives mutating events
- startup answers are validated

Manual/optional:

- FoundationDB provider e2e

## Acceptance criteria

- A realistic landlord/tenant Sorx server can run from `.gtpack`.
- E2E uses HTTP routes generated from pack metadata.
- Memory provider e2e is CI-safe.
- FoundationDB path exists if provider available.
- The scenario proves create/read/update/query and policy/audit/idempotency.
- Docs explain how to run and extend the scenario.

## Codex working style

Complete as much as possible in one pass. Do not fake success. If a capability is missing, document the gap and implement the smallest production feature needed if reasonably scoped.
