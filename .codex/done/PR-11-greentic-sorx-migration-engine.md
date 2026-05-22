# PR: Add migration plan, dry-run and apply commands

Repo: `greenticai/greentic-sorx`

## Goal
Execute SoRLa declarative migrations against canonical provider state.

## Current code assumptions

- There is no `migrate` CLI group yet.
- Deployment registry already has `shared_requires_migration`, which should be the bridge into this PR.
- Validation reports and promotion gates already exist; migration readiness should integrate with them rather than inventing a second deployment lifecycle.
- Canonical state namespace from PR 07 should be `sorx/{tenant}/{sor}`.

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
