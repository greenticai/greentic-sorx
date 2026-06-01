# PR: Extend SORX capability metadata, invocation adapters, event metadata, and policy coverage

## Summary

Extend the existing SORX runtime so each deployed SorLa pack can:

1. include business-action and event-topic offers in the existing runtime capability metadata
2. expose a capability invocation adapter over the existing `SorxRuntime::invoke` path
3. enrich command-emitted events with capability/topic metadata while preserving provider event storage
4. apply the existing risk, approval, authorization, audit, idempotency, control-hook, and observer-hook path consistently across HTTP, MCP, manager submit, and capability invocation

HTTP must remain fully supported as an external ingress and compatibility adapter.

## Motivation

OPERAX should be able to call SORX business actions through capability bindings instead of public HTTP URLs. SORX already has an internal runtime execution path (`SorxRuntime::invoke`) used by HTTP, MCP, and manager submit flows; this work should add a capability adapter to that path rather than introduce a second executor. Command `emit_event` steps already append events through the canonical provider, so event work should add declarative capability metadata and an interoperable envelope at the adapter boundary without breaking existing event records.

## Proposed architecture

```text
HTTP route adapter --------\
MCP adapter ---------------> EndpointInvocation -> SorxRuntime::invoke(...) -> validation -> auth/policy/approval -> provider -> audit -> observer events
Manager submit adapter ----/
Capability adapter --------/
```

No adapter may bypass the current runtime path:

- input validation
- provider resolution
- authorization policy checks
- approval/risk checks
- idempotency handling
- audit logging
- output validation
- command event append / observer emission

## New surfaces

### CLI

```bash
greentic-sorx runtime-host manifest
greentic-sorx inspect <pack.gtpack> --json
greentic-sorx routes <pack.gtpack> --json
greentic-sorx mcp-tools <pack.gtpack>
```

Do not add duplicate top-level `capabilities`, `policy explain`, or `invoke` commands unless there is a separate CLI design PR. The existing CLI already exposes runtime-host capability metadata, pack inspection, route listing, MCP tool listing, and HTTP business-action invoke endpoints. Any new local smoke surface should either extend one of those commands or add a narrowly scoped subcommand under an existing command group.

### Rust API

```rust
pub fn invoke(&self, invocation: EndpointInvocation) -> SorxResult<EndpointResult>;
```

```rust
pub struct EndpointInvocation {
    pub tenant_id: String,
    pub endpoint_id: String,
    pub operation_id: String,
    pub input: serde_json::Value,
    pub caller_id: String,
    pub roles: Vec<String>,
    pub idempotency_key: Option<String>,
}
```

Implementation should add a small adapter that resolves a capability binding to an existing endpoint/business-action version, then constructs `EndpointInvocation` and calls `SorxRuntime::invoke`. Avoid adding an async `SorxInvoker` trait or a parallel `InvocationContext` unless the runtime itself is first made async in a broader API migration.

### Capability export

Build on the current `greentic.capabilities.v1` runtime metadata:

- Keep the existing runtime-host offer (`greentic.cap.runtime.host.v1`) and runtime contracts.
- Add pack-derived business-action offers from loaded `BusinessAction` metadata.
- Add event-topic offers from declared command `emit_event` steps and/or future explicit SorLa event metadata.
- Include enough endpoint/action/version/contract-hash metadata for adapters to resolve to an `EndpointInvocation`.
- Treat HTTP/MCP route details as adapter metadata, not as the primary capability contract.

### Business event envelope

```json
{
  "event_id": "evt_...",
  "event_type": "boiler.work_order_assigned",
  "capability": "cap://greentic/events/boiler/v1/work-order-assigned",
  "producer": "sorx:boiler-maintenance:v1",
  "tenant": "default",
  "team": "maintenance",
  "subject": { "type": "work_order", "id": "wo_123" },
  "correlation_id": "...",
  "causation_id": "...",
  "occurred_at": "...",
  "payload": {}
}
```

This envelope should be produced at the capability/event adapter boundary or included as event metadata. The provider-level event append path currently stores `AppendEventOp`/`EventRecord` data from `CommandStep::EmitEvent`; do not replace that storage contract without a migration. If command events need new fields, add them compatibly and preserve existing tests around event append/query behavior.

## Role/policy enforcement

Do not add a separate role/policy engine. Extend the existing `PolicyEngine` / `AuthorizationPolicyInput` boundary and the existing runtime invoke flow.

```rust
pub struct AuthorizationPolicyInput {
    pub principal_subject: String,
    pub principal_roles: Vec<String>,
    pub resource: AuthorizationPolicyResource,
    pub operation: String,
    pub policies: Vec<String>,
    pub conditions: Option<Value>,
}
```

If event publish/subscribe authorization is needed, add `AuthorizationPolicyResource` variants or a compatible resource model to the existing policy module, then call it from the same runtime path. Decision values should continue to map to `PolicyAction::{Execute, RequireApproval, Deny}` unless a broader policy API change is approved.

SorLa roles/policies should be normalized into the existing `EndpointInvocation.roles`, endpoint authorization policies, record access policies, manager policy view model, and audit details. Do not create a second role model that HTTP/MCP cannot use.

## Secrets/config

- SORX already rejects inline secret-like provider config outside local/test and prefers `config_ref` / `secret://` references in startup answers.
- Runtime host config is loaded from `greentic.runtime-config.v1` files discovered in the current local/environment paths.
- Do not assume direct dependencies on `greentic-secrets` or `greentic-config`; they are advertised as optional runtime capabilities (`greentic.cap.secrets.v1`) and should be integrated through existing config refs/capability bindings when available.
- Keep rejecting inline secret-like values in startup answers.

## HTTP adapter guarantee

HTTP routes are already adapters over `SorxRuntime::invoke` for standard endpoints and business-action invoke routes. Capability invocation is additive and should share that existing internal executor. Do not delete or rename existing HTTP paths such as:

- `POST /v1/sorx/business-actions/{id}/versions/{version}/dry-run`
- `POST /v1/sorx/business-actions/{id}/versions/{version}/invoke`
- generic runtime/admin endpoints under `/admin/v1/...`

## Acceptance criteria

- `greentic-sorx runtime-host manifest` / `/admin/v1/capabilities` keep existing runtime-host capability output and include any new pack-derived offers without breaking the `greentic.capabilities.v1` schema.
- Capability invocation resolves to an existing endpoint/action version and calls `SorxRuntime::invoke` with `EndpointInvocation`.
- HTTP, MCP, manager submit, and capability invocation produce equivalent business results for the same authorized operation.
- Unauthorized capability invocation is denied by the same `PolicyEngine` / authorization path as HTTP/MCP.
- Command-emitted events remain queryable through the existing provider event storage and expose the canonical business event envelope where capability subscribers need it.
- SORX keeps using `config_ref` / secret refs and rejects inline secret-like values outside local/test.

## Test plan

- Unit tests for pack-derived capability offer generation from existing business-action and command metadata.
- Golden tests for `greentic.capabilities.v1` output, including the existing runtime-host offer.
- Capability adapter vs HTTP business-action parity tests using `SorxRuntime::invoke`.
- Policy allow/deny tests through the existing `PolicyEngine`, `AuthorizationPolicyInput`, and `EndpointInvocation.roles`.
- Idempotency tests proving capability invocation uses the same idempotency path as HTTP/MCP.
- Event envelope validation tests plus regression tests that existing provider event append/query behavior still works.
