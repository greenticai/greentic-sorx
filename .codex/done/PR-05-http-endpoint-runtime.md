# PR 05 — HTTP Endpoint Runtime Generated from `agent-gateway.json`

## Goal

Start an HTTP server that exposes agent endpoint routes declared by the SoRLa `.gtpack`.

Routes must be generated from `agent-gateway.json`, not hard-coded for landlord/tenant.

## CLI

Implement:

```bash
greentic-sorx start landlord.gtpack --answers sorx.answers.json
```

This should:

1. load pack
2. validate pack
3. load/validate answers
4. build runtime config
5. build provider registry
6. start HTTP server

Keep `run` as alias only if desired:

```bash
greentic-sorx run landlord.gtpack --answers sorx.answers.json
```

## Server

Use the existing Greentic HTTP runtime convention if one exists. If not, use `axum`.

This PR should keep the route builder ready for the deployment registry introduced in PR 12. Do not bake a singleton route table into global state. Route registration must be keyed by a deployment identity so multiple versions can later be mounted concurrently. Before PR 12 lands, a single implicit local deployment is acceptable, but the public types should already contain `deployment_id`, `pack_name`, `pack_version`, and `pack_digest` fields.

Required system routes:

```text
GET /healthz
GET /readyz
GET /v1/sorx/routes
GET /v1/sorx/tools
GET /v1/sorx/deployments/local/routes
```

Generated endpoint routes should come from gateway metadata.

Example:

```text
POST /v1/agent/tenants/create
GET  /v1/agent/tenants/{tenant_id}
PATCH /v1/agent/tenants/{tenant_id}
POST /v1/agent/tenancies/assign
POST /v1/agent/payments/record
POST /v1/agent/maintenance/open
```

Do not hard-code these exact routes except in fixtures.

## Request lifecycle

Implement:

```text
HTTP request
  → route match
  → caller context extraction
  → tenant context extraction
  → JSON body parse
  → input schema validation
  → EndpointInvocation
  → RuntimeCore.execute()
  → structured JSON response
```

## Headers

Support:

```text
X-Greentic-Tenant-Id
X-Greentic-Caller-Id
X-Greentic-Caller-Role
Idempotency-Key
```

For local mode, allow tenant/caller fallback from answers if headers are missing.

For non-local mode, missing tenant/caller should fail.

## Response shape

Success:

```json
{
  "ok": true,
  "endpoint_id": "tenant.create",
  "operation_id": "tenant.create",
  "result": {
    "id": "tenant_123",
    "full_name": "Sarah Ahmed"
  },
  "events": []
}
```

Error:

```json
{
  "ok": false,
  "error": {
    "code": "SORX_VALIDATION_FAILED",
    "message": "Missing required field full_name",
    "details": {}
  }
}
```

## Route listing

Implement:

```bash
greentic-sorx routes landlord.gtpack
greentic-sorx routes --deployment local
```

and HTTP:

```text
GET /v1/sorx/routes
```

Output:

```json
{
  "schema": "greentic.sorx.routes.v1",
  "routes": [
    {
      "method": "POST",
      "path": "/v1/agent/tenants/create",
      "endpoint_id": "tenant.create",
      "operation_id": "tenant.create",
      "risk": "medium",
      "deployment_id": "local",
      "pack_name": "landlord-tenant-sor",
      "pack_version": "0.1.0",
      "exposure": "local"
    }
  ]
}
```

## Tests

Add integration tests using the in-memory provider:

- server starts with valid pack/answers
- `/healthz` returns ok
- `/readyz` returns ok
- route list matches gateway
- route list includes deployment and pack identity fields
- create tenant via HTTP
- get tenant via HTTP
- update tenant via HTTP
- query tenants via HTTP
- invalid JSON returns structured error
- invalid schema input returns structured error
- missing tenant header fails outside local mode
- idempotency key prevents duplicate create

## Acceptance criteria

- Sorx can start an HTTP server from `.gtpack` + answers.
- Routes come from `agent-gateway.json`.
- Runtime path uses the core router from PR 04.
- Responses are structured.
- Tests cover successful and failed calls.
- Landlord routes are fixture-driven, not hard-coded.

## Codex working style

Complete as much as possible in one pass. Use existing HTTP conventions if available; otherwise use Axum with clean adapter boundaries.
