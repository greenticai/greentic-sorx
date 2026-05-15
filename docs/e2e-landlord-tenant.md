# Landlord/Tenant E2E

The PR 09 e2e scenario lives in the CLI crate as
`tests/landlord_tenant_e2e.rs` with deterministic JSON fixtures under
`tests/e2e/fixtures/landlord_tenant`.

Run the CI-safe memory-provider scenario with:

```bash
bash scripts/e2e/run-landlord-tenant.sh --provider memory
```

The test builds a deterministic `.gtpack` in a temp directory, runs
`greentic-sorx doctor`, runs `greentic-sorx inspect --json`, starts
`greentic-sorx start --answers` on a local port, and drives the generated HTTP
routes for:

- landlord, property, unit, tenant, tenancy, payment, and maintenance creation
- active tenant query
- tenant contact preference update
- record read-back
- mutating idempotency
- high-risk approval-required policy behavior
- MCP tool metadata listing
- locked business action listing, id/version lookup, dry-run, invoke, audit
  metadata, and contract-hash rejection

The embedded business action fixture uses:

```text
record_rent_payment@0.1.0 -> payment.record
```

The runtime call uses:

```http
POST /v1/sorx/business-actions/record_rent_payment/versions/0.1.0/invoke
```

The e2e proves that dry-run does not create the payment record, while invoke
uses the generated endpoint runtime path after id/version/hash and payload
validation pass.

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
