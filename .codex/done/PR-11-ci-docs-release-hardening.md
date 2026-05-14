# PR 11 — CI, Documentation, Security Hardening, and Release Readiness

## Goal

Make `greentic-sorx` maintainable, secure, and ready for real use.

This PR focuses on CI workflows, documentation, local checks, security scanning, deterministic outputs, and release packaging.

## CI workflows

Add or update workflows:

```text
.github/workflows/ci.yml
.github/workflows/e2e.yml
.github/workflows/release.yml
```

CI should run:

- cargo fmt
- cargo clippy
- cargo test
- pack doctor tests
- startup answer tests
- HTTP runtime tests
- memory provider e2e
- docs link check if existing convention exists

E2E workflow:

```yaml
workflow_dispatch:
  inputs:
    scenario:
      description: "E2E scenario"
      default: "landlord-tenant"
    provider:
      description: "memory | foundationdb"
      default: "memory"
```

FoundationDB should be manual/optional unless easy and stable in CI.

## Local checks

Add or align with existing local check script:

```bash
./scripts/local_check.sh
```

It should run the same core checks as CI.

If `greentic-dev` has local check conventions, reuse them.

## Security hardening

Add checks to ensure:

- `.gtpack` input paths cannot escape archive boundaries
- path traversal blocked
- no secrets embedded in packs or emitted normalized answers by default
- provider `config_ref` preferred over inline config
- critical operations denied by default
- high-risk mutations require approval by default
- request bodies are not logged unless explicitly enabled
- tenant ID required outside local mode
- caller context required outside local mode
- idempotency recommended or required for mutating operations
- pack digest/signature fields are recognised for future validation

Add security doc:

```text
docs/security.md
```

## Observability docs

Document structured events:

- pack loaded
- route registered
- tool registered
- request received
- policy decision
- approval requested
- provider operation started/completed
- request failed

Add examples.

## User docs

Add:

```text
docs/getting-started.md
docs/commands.md
docs/answers.md
docs/provider-bindings.md
docs/mcp.md
docs/e2e-landlord-tenant.md
docs/future-signing-and-versioning.md
```

README should include the minimal flow:

```bash
greentic-sorla pack examples/landlord.sorla --out landlord.gtpack
greentic-sorx doctor landlord.gtpack
greentic-sorx start landlord.gtpack --schema > sorx.schema.json
greentic-sorx start landlord.gtpack --answers examples/landlord.answers.json
```

## Release

If Greentic uses toolchain manifests, document how `greentic-sorx` should be added later.

Do not publish unless explicitly configured.

Add release metadata:

- crate/bin name: `greentic-sorx`
- semantic versioning
- changelog
- expected install path
- future GHCR/toolchain manifest inclusion

## Determinism

Ensure:

- JSON output stable
- route listing stable
- tool listing stable
- doctor report stable
- generated normalized answers stable
- tests do not depend on current time except where injected
- no absolute paths in stable outputs unless explicitly requested

## Tests

Add tests:

- stable output snapshots
- security checks
- no request body in audit by default
- path traversal rejected
- critical operation denied default
- high-risk requires approval default
- normalized answers do not include secret values where marked secret
- local check script passes

## Acceptance criteria

- CI runs core checks.
- Manual e2e workflow exists.
- Security model is documented and tested.
- User docs explain the full flow.
- Release/toolchain integration is documented.
- Stable JSON outputs are tested.
- Local check script exists or integrates with existing convention.

## Codex working style

Complete as much as possible in one pass. Do not publish packages or push release tags. Routine CI, docs, tests, and hardening are pre-approved.
