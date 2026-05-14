# Deployment Registry

PR12 adds a local deployment registry so SORX can represent multiple immutable
pack artifacts for the same tenant and SOR name.

The local registry is JSON:

```json
{
  "schema": "greentic.sorx.deployment-registry.v1",
  "deployments": [],
  "aliases": []
}
```

Use `--registry <path>` or `SORX_REGISTRY_PATH` to choose the registry file.
Without either value, SORX uses the user config directory.

```bash
greentic-sorx --registry /tmp/sorx-registry.json deployments create \
  --pack landlord.gtpack \
  --tenant acme \
  --sor landlord-tenant \
  --environment production \
  --api-version v1.1 \
  --base-path /sorx/acme/landlord-tenant/v1.1 \
  --visibility private

greentic-sorx --registry /tmp/sorx-registry.json deployments validate <deployment-id>
greentic-sorx --registry /tmp/sorx-registry.json deployments activate <deployment-id> --private
greentic-sorx --registry /tmp/sorx-registry.json deployments promote <deployment-id> --public
greentic-sorx --registry /tmp/sorx-registry.json deployments promote <deployment-id> --alias preview
greentic-sorx --registry /tmp/sorx-registry.json deployments promote <deployment-id> --alias latest --public
greentic-sorx --registry /tmp/sorx-registry.json aliases set \
  --tenant acme \
  --sor landlord-tenant \
  --alias stable \
  --target <deployment-id>
```

Routes can be listed for a deployment:

```bash
greentic-sorx --registry /tmp/sorx-registry.json routes --deployment <deployment-id> --json
greentic-sorx --registry /tmp/sorx-registry.json deployments public-routes
greentic-sorx --registry /tmp/sorx-registry.json deployments promotion-status <deployment-id>
```

Public promotion is gated by the latest validation report for the same
deployment ID and pack digest. The report must have `result=pass` and
`public_exposure_allowed=true`; stale reports for older digests are rejected.
Promotion writes an audit event into the registry before exposing the route or
moving an alias.

For ontology-enabled validation reports, promotion status also reports
per-gate ontology checks: `ontology-static`, `provider-compatibility`,
`retrieval-bindings`, and `ontology-policy`. All ontology gates must pass unless
a local operator policy override is recorded in the validation report. Overrides
are surfaced as a `local-operator-override` gate and promotion still writes a
registry audit event.

Private activation only requires the validation report to be present, match the
pack digest, and pass. Ontology public-exposure gates do not block private
activation.

Rollback is alias-based and does not delete the failed deployment:

```bash
greentic-sorx --registry /tmp/sorx-registry.json deployments rollback \
  --tenant acme \
  --sor landlord-tenant \
  --alias latest \
  --to <previous-deployment-id> \
  --reason "failed smoke validation"

greentic-sorx --registry /tmp/sorx-registry.json deployments retire-old \
  --tenant acme \
  --sor landlord-tenant \
  --keep 3
```

Current states:

- `installed`
- `pending`
- `validating`
- `validated`
- `active_private`
- `active_public`
- `failed`
- `failed_public_promotion`
- `retired`
- `rolled_back`

The local HTTP runtime exposes read-only diagnostics for
`GET /v1/sorx/public-routes` and
`GET /v1/sorx/deployments/{deployment_id}/promotion-status`. Mutating HTTP
admin endpoints remain disabled unless a future build wires admin auth and
registry storage into the runtime.
