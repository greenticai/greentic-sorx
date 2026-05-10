# Landlord/Tenant E2E

The PR 09 e2e scenario lives in the CLI crate as
`tests/landlord_tenant_e2e.rs` with deterministic JSON fixtures under
`tests/e2e/fixtures/landlord_tenant`.

Run the CI-safe memory-provider scenario with:

```bash
bash scripts/e2e/run-landlord-tenant.sh --provider memory
```

The test builds a deterministic `.gtpack` in a temp directory, runs
`greentic-sorx doctor`, starts `greentic-sorx start --answers` on a local port,
and drives the generated HTTP routes for:

- landlord, property, unit, tenant, tenancy, payment, and maintenance creation
- active tenant query
- tenant contact preference update
- record read-back
- mutating idempotency
- high-risk approval-required policy behavior
- MCP tool metadata listing

The checked-in FoundationDB answers fixture is present for manual expansion:

```bash
bash scripts/e2e/run-landlord-tenant.sh --provider foundationdb
```

That path currently verifies only that the FoundationDB fixture exists. Real
FoundationDB execution is blocked until `greentic-sorla-providers` exposes a
SORX-compatible CRUD store adapter; see
`docs/audit/provider-foundationdb-gap-pr08.md`.

Schema migration is not part of this scenario yet. It should be added after SORX
has first-class schema version handling.
