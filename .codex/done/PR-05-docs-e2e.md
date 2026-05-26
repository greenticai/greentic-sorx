# PR-05 — Add SORX metrics docs and end-to-end tests

Repository: `greenticai/greentic-sorx`

## Goal

Document and test runtime support for declared SoRLa metrics.

## Current repo validation

Existing docs and e2e patterns to reuse:

- Docs live under `docs/`, with feature-specific pages such as `docs/business-actions.md`, `docs/e2e-landlord-tenant.md`, and `docs/provider-bindings.md`.
- CLI integration tests build temporary `.gtpack` files in `crates/greentic-sorx-cli/tests/cli_smoke.rs`.
- Longer e2e fixture files live under `crates/greentic-sorx-cli/tests/e2e/fixtures/`.
- The runtime HTTP harness is currently covered through unit tests in `crates/greentic-sorx-cli/src/http_runtime.rs`.

Design update:

- Add docs only after PR-01 through PR-04 have landed, so examples match real schemas and routes.
- Prefer generated test `.gtpack` fixtures or small JSON fixture files over committing opaque binary `.gtpack` artifacts.
- If an HTTP server is required for e2e, reuse the existing test harness style where possible instead of starting a fragile long-running process.
- Keep packs without metrics in the regression suite.

## Docs

Add:

```text
docs/metrics-runtime.md
```

Cover:

- how metrics are loaded from `.gtpack`
- doctor/inspect behavior
- HTTP routes
- MCP tools
- provider capability requirements
- query examples
- formula metric limitations
- audit and policy behavior

## E2E fixture

Use a pack with:

- click events
- payments
- costs
- daily_clicks
- monthly_revenue
- monthly_cost
- gross_margin

Test:

```bash
greentic-sorx doctor metrics-commerce.gtpack --json
greentic-sorx inspect metrics-commerce.gtpack
greentic-sorx start metrics-commerce.gtpack --answers metrics-commerce.sorx.answers.json
```

Then query metrics via HTTP or internal test harness.

## Acceptance criteria

- Docs explain the runtime metrics model.
- E2E tests cover loading and querying metrics.
- Packs without metrics still pass existing tests.
- Invalid metrics produce useful doctor diagnostics.
