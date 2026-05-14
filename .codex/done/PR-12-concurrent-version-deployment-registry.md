# PR 12 — Concurrent Version Deployment Registry

## Goal

Add a first-class SORX deployment registry so multiple versions of the same SoRLa `.gtpack` can be deployed concurrently for the same tenant and exposed through separate versioned APIs.

SORX must stop thinking in terms of "the running pack" and start thinking in terms of immutable pack artifacts mounted as deployment instances.

## Core model

Introduce these concepts:

```text
PackArtifact
  immutable .gtpack reference: source, name, version, digest, signature metadata

SorxDeployment
  runtime instance of one pack artifact for one tenant/environment/version label

DeploymentAlias
  mutable pointer such as stable/latest/preview to one deployment

DeploymentRouteTable
  generated route table scoped to one deployment
```

The deployment identity must include:

```text
tenant_id
sor_name
pack_name
pack_version
pack_digest
deployment_id
api_version_label
environment
```

## Required deployment states

Implement a small state machine:

```text
installed       artifact known but not configured
pending         deployment config created, not validated
validating      pack and validation suite running
validated       validation passed, safe for internal/private use
active_private  endpoint mounted but not public
active_public   endpoint mounted and public
failed          validation/start failed
retired         no longer routable
rolled_back     alias moved away after failure
```

Only `active_public` may be exposed externally.

## Registry storage

Start with a durable local JSON/CBOR registry under the SORX config directory. Keep an interface that can later be backed by greentic-state or a remote control plane.

Example shape:

```json
{
  "schema": "greentic.sorx.deployment-registry.v1",
  "deployments": [
    {
      "deployment_id": "acme-landlord-v1-0-0-sha256abc",
      "tenant_id": "acme",
      "sor_name": "landlord-tenant",
      "pack_name": "landlord-tenant-sor",
      "pack_version": "1.0.0",
      "pack_digest": "sha256:abc",
      "environment": "production",
      "api_version_label": "v1",
      "base_path": "/sorx/acme/landlord-tenant/v1",
      "state_namespace": "sorx/acme/landlord-tenant/1.0.0",
      "visibility": "private",
      "status": "validated"
    }
  ],
  "aliases": [
    {
      "tenant_id": "acme",
      "sor_name": "landlord-tenant",
      "alias": "stable",
      "target_deployment_id": "acme-landlord-v1-0-0-sha256abc"
    }
  ]
}
```

## CLI

Add:

```bash
greentic-sorx deployments list

greentic-sorx deployments inspect <deployment-id>

greentic-sorx deployments create   --pack landlord-tenant-sor.gtpack   --tenant acme   --sor landlord-tenant   --environment production   --api-version v1.1   --base-path /sorx/acme/landlord-tenant/v1.1   --visibility private

greentic-sorx deployments validate <deployment-id>

greentic-sorx deployments activate <deployment-id> --private

greentic-sorx deployments retire <deployment-id>

greentic-sorx aliases set   --tenant acme   --sor landlord-tenant   --alias stable   --target <deployment-id>

greentic-sorx aliases list --tenant acme
```

## HTTP admin API

Add internal-only endpoints:

```text
GET  /v1/sorx/deployments
GET  /v1/sorx/deployments/{deployment_id}
POST /v1/sorx/deployments
POST /v1/sorx/deployments/{deployment_id}/validate
POST /v1/sorx/deployments/{deployment_id}/activate-private
POST /v1/sorx/deployments/{deployment_id}/retire
GET  /v1/sorx/aliases
PUT  /v1/sorx/aliases/{tenant_id}/{sor_name}/{alias}
```

These endpoints must be disabled unless admin API is explicitly enabled.

## Route generation

Generated endpoint routes must be scoped by deployment:

```text
/sorx/{tenant}/{sor}/{api_version}/...
/sorx/{tenant}/{sor}/stable/...
/sorx/{tenant}/{sor}/preview/...
```

Alias routes resolve to a deployment at request time. Version routes resolve directly.

## Conflict checks

Reject a deployment if:

- deployment ID already exists
- base path conflicts with an active deployment
- alias would point across tenant or SOR name
- pack digest does not match the loaded artifact
- requested API version label is already active for a different digest unless explicitly allowed
- state namespace conflicts without explicit shared-state compatibility

## Shared-state compatibility

Represent state mode:

```text
isolated
shared_compatible
shared_requires_migration
```

Default to isolated state for new versions. Shared state requires explicit compatibility metadata from the pack or explicit admin override.

## Tests

Add tests for:

- creating two deployments for the same pack name with different versions
- creating two deployments for same version but different digest is rejected unless forced with a different API label
- route tables remain separate
- aliases resolve to the correct deployment
- retiring one version does not affect the other
- base path conflict detection
- state namespace conflict detection
- local registry survives restart

## Acceptance criteria

- SORX can represent multiple active deployments for the same SoR name.
- Routes are deployment-scoped, not singleton/global.
- Aliases are mutable pointers to immutable deployment IDs.
- No public endpoint exposure happens in this PR; public promotion is added in PR 15.

## Codex working style

Implement a clean domain model and tests first. Keep storage simple but abstracted. Avoid broad rewrites unless needed to remove singleton route/runtime assumptions.
