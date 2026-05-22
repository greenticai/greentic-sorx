# PR: Extend deployment registry for concurrent active versions

Repo: `greenticai/greentic-sorx`

## Goal
Finish multi-version deployment support by aligning the existing deployment registry with shared canonical state and explicit view/canonical version metadata.

## Current code assumptions

- The registry already allows multiple active deployments for the same `tenant_id` + `sor_name` when base paths do not collide.
- Aliases already exist and are optional pointers to routable deployments.
- Deployment-scoped route listing already exists.
- Existing field names are `tenant_id`, `sor_name`, `pack_name`, `pack_version`, `api_version_label`, `base_path`, `state_mode` and `state_namespace`.
- Existing state modes are `isolated`, `shared_compatible` and `shared_requires_migration`; do not introduce `shared_canonical` without migrating the enum and CLI parser.

## Registry fields

```json
{
  "deployment_id": "dep-a",
  "tenant_id": "acme",
  "sor_name": "landlord-tenant",
  "pack_name": "leasing-app",
  "pack_version": "1.1.0",
  "pack_digest": "sha256:...",
  "environment": "prod",
  "api_version_label": "v1.1",
  "canonical_version": "2.0.0",
  "base_path": "/sorx/acme/landlord-tenant/v1.1",
  "state_mode": "shared_compatible",
  "state_namespace": "sorx/acme/landlord-tenant",
  "status": "active_private"
}
```

## Acceptance criteria

- Preserve existing support for multiple active deployments when base paths do not collide.
- Preserve existing alias behavior as optional pointers, not the only active version mechanism.
- Add explicit canonical/view version fields to deployment records or a compatible metadata extension.
- Route diagnostics include deployment id, API/view version, canonical version and state namespace.
- `public-routes` includes the same version metadata for promoted deployments.
- Existing local JSON registries either migrate cleanly or deserialize with sensible defaults.
