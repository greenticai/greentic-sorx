# PR: Add migration plan, dry-run and apply commands

Repo: `greenticai/greentic-sorx`

## Goal
Execute SoRLa declarative migrations against canonical provider state.

## Current code assumptions

- There is already a `migrate` CLI group with `plan`, `dry-run`, and `apply` subcommands in `crates/greentic-sorx-cli/src/lib.rs`.
- The current migration implementation is intentionally lightweight: it builds deterministic JSON plans, validates answers, writes a sidecar `*.status.json`, rejects destructive plans unless `--allow-destructive` is passed, and treats reapplying a completed sidecar status as idempotent.
- There is not yet a full provider-backed migration executor. Status is not currently stored in canonical provider state.
- Deployment registry already has `StateMode::SharedRequiresMigration`, but promotion gating is not yet wired to a concrete migration completion record.
- Validation reports and promotion gates already exist; migration readiness should integrate with them rather than inventing a second deployment lifecycle.
- Canonical state namespace from PR 07 should be `sorx/{tenant}/{sor}`.

## Design update

Do not re-add the CLI group. The remaining design work is to evolve the existing command surface into real canonical-state execution:

- Add provider-backed migration status storage behind the canonical store abstraction.
- Connect `shared_requires_migration` promotion gating to completed migration status or explicit waiver policy.
- Keep the deterministic JSON plan/dry-run/status shapes stable unless a schema version changes.
- Keep destructive steps disabled by default.
- Treat the existing sidecar status file as a local/dev fallback, not the final production source of truth.

## CLI

```bash
greentic-sorx migrate plan --from old.gtpack --to new.gtpack --tenant acme --sor landlord-tenant --out plan.json
greentic-sorx migrate dry-run --plan plan.json --answers sorx.answers.json
greentic-sorx migrate apply --plan plan.json --answers sorx.answers.json
```

## Behaviour

- Migrations are idempotent.
- Migration status is stored in canonical provider state under `sorx/{tenant}/{sor}/migrations/{migration_id}` or an equivalent typed collection.
- Applying an already-completed migration is a no-op.
- Destructive migration steps require explicit policy flag and are initially disabled.
- Deployments in `shared_requires_migration` cannot be promoted to shared active state until the required plan has been applied or explicitly waived by policy.

## Acceptance criteria

- Additive field migration works.
- Index-build migration works.
- Split-entity fixture can be planned and dry-run, even if apply is initially limited.
- Migration plan/dry-run output can be attached to deployment validation or promotion diagnostics.
