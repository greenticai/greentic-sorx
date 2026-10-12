# Agent-endpoint capability discovery + invoke — design (epic #6 follow-up)

_Date: 2026-07-27 · Repo: `greentic-sorx` · Branch: `feat/agent-endpoint-capabilities` off `research`_

## Context

Correction to an earlier audit: SoRLa **agent-endpoints ARE already served at runtime** — `agent_gateway_json`
(built 1:1 from `ir.agent_endpoints` by greentic-sorla-pack) is the source for
`EndpointRouter::from_agent_gateway` in the production constructor
(`http_runtime.rs:175`), so each agent-endpoint compiles to a real `EndpointDefinition` reachable at
its own HTTP route (e.g. `POST /v1/agent/tenants/create`). The gap is NARROW: agent-endpoints are NOT
advertised in `GET /admin/v1/capabilities` nor invocable via `POST /admin/v1/capabilities/invoke`
(which today resolve ONLY `BusinessAction`s + event topics). This adds agent-endpoints to that
capability surface, mirroring `business_action_offers`/`find_business_action_by_capability`.

`backing.actions` is pure lineage metadata (never in the execution path) — invoking an agent-endpoint
routes through its OWN compiled `EndpointDefinition`, not through any backing action. So this covers
ALL agent-endpoints uniformly, not just single-action-backed ones.

## Non-goals

- No new execution engine / flow orchestration — reuse `self.runtime.invoke(EndpointInvocation)`.
- No change to the direct `/v1/agent/...` HTTP routes (already work).
- No `greentic-sorla` change (the `agent_gateway_json` already carries everything needed).
- No fix to the `sorx_runtime_method`/`sorx_runtime_path` id→CRUD-verb heuristic — it's a pre-existing
  condition of the live routes, out of scope.

## Design (`crates/greentic-sorx-cli/src/http_runtime.rs`)

### Capability URI

New namespace: `cap://greentic/agent-endpoints/<pack>/<endpoint_id>/v<pack_version>` — parallel to
`business_action_capability`'s `cap://greentic/business-functions/<pack>/<id>/v<version>`. A small
builder `agent_endpoint_capability(pack_name, endpoint_id, pack_version)` mirroring
`business_action_capability`, using the same `clean_capability_segment` sanitization.

### 1. Discovery — `agent_endpoint_offers(&self) -> Vec<CapabilityOffer>`

Source the endpoint list from the agent-gateway manifest. `runtime_capabilities()` (`:1107`) currently
reads `self.business_actions`; `agent_gateway_json` is used at construction (`:175`, `:233`) — CONFIRM
whether it (or its parsed endpoints) is reachable from `&self`; if NOT stored, store the parsed
`agent_gateway_json` (or its `endpoints: Vec<AgentGatewayEndpointRef>` + `pack.name`/`pack.version`)
on `HttpRuntime` at construction, the same way `business_actions` is stored (`:275`). Then, for each
endpoint, emit a `CapabilityOffer`:
- `capability`: `agent_endpoint_capability(pack, endpoint.endpoint_id, pack_version)`
- `contracts`: `["greentic.sorx.agent-endpoint.invoke.v1"]`
- `metadata`: `{ "kind": "agent_endpoint", "pack": {name,version}, "endpoint": { id, title, intent,
  risk, approval, method, path, operation_id }, "input_schema", "output_schema", "exports": {mcp,...} }`
  (fields available on `AgentGatewayEndpointRef` / the manifest — use what's present; omit absent).

Extend `runtime_capabilities()` to `.extend(self.agent_endpoint_offers())` after the business-action +
event-topic offers.

### 2. Resolver — `find_router_endpoint_by_agent_endpoint_capability(&self, capability: &str) -> Option<(&EndpointDefinition, ...)>`

Parse a `cap://greentic/agent-endpoints/<pack>/<endpoint_id>/v<ver>` URI (guard segment count; wrong
namespace / malformed → None). Verify the `<pack>` matches this runtime's pack (cleaned), then
`self.runtime.router.endpoints.get(endpoint_id)` (the same router `execution_endpoint` uses). Return
the endpoint (+ its `operation_id`) needed to build an `EndpointInvocation`.

### 3. Invoke branch — in `invoke_capability` (`:1258`)

Today it calls `find_business_action_by_capability`. Add: if that returns `None` AND the capability is
in the `agent-endpoints` namespace, resolve via (2), build
`EndpointInvocation { endpoint_id, operation_id, input, caller, idempotency_key, source }` (mirroring
the business-action path's construction), call `self.runtime.invoke(invocation)`, and build the
success response. Use a DISTINCT, simpler result shape for agent-endpoints (no `action_ref`
contract-hash lock, no `idempotency` struct — those are BusinessAction-only; the response should carry
the endpoint id/version + the invoke result + events, schema e.g.
`greentic.sorx.agent-endpoint-invoke-result.v1`). Preserve the existing approval/risk gating +
error mapping (`202 approval_required` / `403 Denied` / `404` capability-not-found) that
`invoke_capability` already applies via `self.runtime.policy`/`invoke`. A capability that matches
NEITHER a business-action NOR an agent-endpoint → the existing `404 RUNTIME_CAPABILITY_NOT_FOUND`.

Auth/gating is unchanged — `invoke_capability` already runs behind `authorize()` + the same
approval/policy path; the agent-endpoint branch reuses it.

## Testing

- **Discovery**: extend the existing `/admin/v1/capabilities` test — assert an `agent_endpoint`-kind
  offer appears with the `cap://greentic/agent-endpoints/<pack>/<id>/v<ver>` URI + the metadata
  (mirror the business-action offer assertion). Use a fixture pack that has agent_endpoints (the
  live `POST /v1/agent/...` tests already use one — reuse that pack).
- **Invoke**: `POST /admin/v1/capabilities/invoke` with an agent-endpoint `cap://` → resolves to the
  same execution as the direct `/v1/agent/...` route + returns the agent-endpoint result shape.
  Assert parity with a direct-route call on the same endpoint (same output). Include: unknown
  agent-endpoint cap → 404; a `dry_run` variant if the business-action path supports it (mirror).
- **cap builder / parser** unit tests: round-trip + malformed/namespace-mismatch → None.

## Files touched

- `crates/greentic-sorx-cli/src/http_runtime.rs` — `agent_endpoint_capability` builder,
  `agent_endpoint_offers`, `find_router_endpoint_by_agent_endpoint_capability`, the `invoke_capability`
  branch + response shape, and (if needed) storing the agent-gateway endpoint list on `HttpRuntime`.
- Possibly `crates/greentic-sorx-core/src/generic_runtime.rs` if a new offer-metadata helper is cleaner
  there (optional).
- No `greentic-sorla` change.

## Global constraints

- Rust edition per repo; `#![forbid(unsafe_code)]`; **no `unwrap()`/`panic!()` in production** (parse
  guards return None; invoke errors map to HTTP status, never panic).
- Reuse `business_action_offers`/`find_business_action_by_capability`/`execution_endpoint`/
  `self.runtime.invoke` patterns line-for-line; do NOT duplicate the router or executor.
- English; Conventional Commits; **NO AI attribution**. `bash ci/local_check.sh` green (fmt + clippy
  `-D warnings` + test; sorx `perf` job is pre-existing-red/non-required — document, don't hide).
- Additive: existing business-action discovery/invoke + direct `/v1/agent/...` routes unchanged.

See epic memory `sorla-sorx-productionization-epic` (followup audit) for the grounding that reversed
the earlier "agent-endpoints not invocable" premise.
