# PR 15 — Public Endpoint Promotion, Rollout, and Rollback Gates

## Goal

Use deployment registry state and validation-suite reports to control when SORX makes versioned endpoints public.

A newly published SORLA `.gtpack` should follow this lifecycle:

```text
GHCR publish webhook
  -> pending deployment
  -> pack doctor
  -> validation suite
  -> active_private
  -> optional active_public promotion
  -> alias update
  -> rollback/retire old version when policy allows
```

## Public exposure rule

SORX must never expose a new endpoint publicly unless all are true:

1. pack doctor passed
2. validation suite passed or policy explicitly allows no suite for that environment
3. provider binding resolved
4. required secrets/config references resolved
5. route conflict checks passed
6. policy permits public exposure
7. admin or automation policy approved promotion
8. audit event was written

## Promotion policies

Implement:

```text
manual_only
validate_then_private
validate_then_public_preview
validate_then_public_alias
```

Policy behavior:

- `manual_only`: webhook creates pending deployment only.
- `validate_then_private`: validation pass activates private endpoint only.
- `validate_then_public_preview`: validation pass exposes only preview alias/path.
- `validate_then_public_alias`: validation pass exposes public route and optionally moves an alias such as `latest`.

Production default must be `manual_only` or `validate_then_private`, not automatic public latest.

## CLI

Add:

```bash
greentic-sorx deployments promote <deployment-id> --public

greentic-sorx deployments promote <deployment-id> --alias preview

greentic-sorx deployments promote <deployment-id> --alias latest --public

greentic-sorx deployments rollback   --tenant acme   --sor landlord-tenant   --alias latest   --to <previous-deployment-id>

greentic-sorx deployments retire-old   --tenant acme   --sor landlord-tenant   --keep 3
```

## HTTP admin API

Add internal-only endpoints:

```text
POST /v1/sorx/deployments/{deployment_id}/promote-public
POST /v1/sorx/deployments/{deployment_id}/promote-alias/{alias}
POST /v1/sorx/deployments/rollback
POST /v1/sorx/deployments/retire-old
```

All mutating endpoints require admin auth integration or a clearly marked local-only mode.

## Public route table

Maintain separate route tables:

```text
private/internal route table
public route table
alias route table
```

Public route table must only include deployments with `status=active_public`.

Expose diagnostics:

```text
GET /v1/sorx/public-routes
GET /v1/sorx/deployments/{deployment_id}/promotion-status
```

## Canary and traffic splitting

Add data model support for canary but keep runtime simple if necessary:

```yaml
traffic:
  mode: all | percent | header
  percent: 10
  header:
    name: X-Greentic-SORX-Canary
    value: v2
```

Initial implementation may only support `all` and `header`; percent routing can be a follow-up if there is no existing router support.

## Rollback

Rollback should be alias-based by default:

```text
latest -> old deployment
```

Do not delete the failed deployment. Mark it:

```text
failed_public_promotion
rolled_back
```

Audit must include:

```text
old_target_deployment_id
new_target_deployment_id
reason
actor
automation_source
```

## Webhook integration

Connect to PR 13:

- successful webhook may create deployment
- if policy is `validate_then_private`, run validation and activate private
- if policy is `validate_then_public_preview`, run validation and expose preview after pass
- if policy is `validate_then_public_alias`, run validation and move the configured alias after pass
- if validation fails, mark deployment failed and do not expose it

## Validation integration

Connect to PR 14:

- public promotion requires a validation report for the same deployment ID and pack digest
- stale validation report for different digest must not be accepted
- if pack changed, validation must rerun
- report must say `public_exposure_allowed=true`

## Tests

Add tests for:

- promotion blocked when validation report missing
- promotion blocked when report digest differs
- promotion blocked when required test failed
- private activation allowed when policy permits and validation passes
- public preview route appears only after promotion
- alias moves to new deployment after promotion
- rollback moves alias back
- old deployment remains active until explicitly retired
- failed validation from webhook does not expose route
- successful webhook + validation + preview policy exposes preview only
- production default does not auto-public latest

## Acceptance criteria

- SORX has an explicit public endpoint lifecycle.
- A new `.gtpack` can be deployed concurrently and validated without affecting existing public routes.
- Public exposure is gated by validation and policy.
- Alias rollback is safe and audited.
- GHCR webhook automation can drive deployment without bypassing validation.

## Codex working style

Keep the public-gate code simple and auditable. Do not couple promotion directly to webhook handling; webhook should request lifecycle transitions through the same deployment service used by CLI/admin API.
