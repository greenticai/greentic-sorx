# PR 06 — Risk Policy, Approval Broker, Idempotency, and Audit Events

## Goal

Enforce runtime risk/approval metadata from SoRLa endpoint definitions before provider-backed operations execute.

Also add structured audit events for all endpoint invocations, policy decisions, approvals, and provider operations.

## Risk model

Define:

```rust
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}
```

Endpoint metadata from `agent-gateway.json` should include or resolve to:

```json
{
  "endpoint_id": "tenancy.terminate",
  "operation_id": "tenancy.terminate",
  "risk": "high",
  "approval": {
    "required": true,
    "roles": ["landlord_admin"],
    "reason_required": true
  }
}
```

If risk metadata is missing for mutating operations, `doctor` should fail or warn depending strictness. Runtime should default to safe policy.

## Policy config

From startup answers:

```json
{
  "policy": {
    "approvals": {
      "low": "auto",
      "medium": "auto",
      "high": "require_approval",
      "critical": "deny"
    }
  }
}
```

Modes:

```text
auto
require_approval
deny
```

## Policy engine

Add:

```rust
pub struct PolicyEngine {
    pub config: PolicyConfig,
}

pub struct PolicyDecision {
    pub action: PolicyAction,
    pub reason: String,
}

pub enum PolicyAction {
    Execute,
    RequireApproval,
    Deny,
}
```

Policy happens before provider execution.

## Approval broker

Add trait:

```rust
#[async_trait]
pub trait ApprovalBroker: Send + Sync {
    async fn decide(&self, request: ApprovalRequest) -> SorxResult<ApprovalDecision>;
}
```

Implement local brokers:

```text
LocalAutoApproveBroker
LocalDenyBroker
LocalPendingBroker
```

For now, `LocalPendingBroker` can return structured pending status rather than blocking forever.

## Pending approval response

For high-risk operations requiring approval, return:

```json
{
  "ok": false,
  "status": "approval_required",
  "approval": {
    "request_id": "approval_...",
    "risk": "high",
    "reason": "Operation requires approval"
  }
}
```

Do not execute provider operation until approved.

## Audit events

Add structured audit sink interface:

```rust
#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn emit(&self, event: SorxAuditEvent) -> SorxResult<()>;
}
```

Implement:

```text
StdoutAuditSink
MemoryAuditSink
DisabledAuditSink
```

Event examples:

```json
{
  "event": "sorx.endpoint.invoked",
  "pack": "landlord-tenant-sor",
  "version": "0.1.0",
  "tenant_id": "demo-landlord",
  "endpoint_id": "tenant.create",
  "operation_id": "tenant.create",
  "risk": "medium",
  "caller_id": "agent.local",
  "decision": "executed",
  "duration_ms": 41
}
```

Events:

- `sorx.pack.loaded`
- `sorx.route.registered`
- `sorx.endpoint.invoked`
- `sorx.policy.decided`
- `sorx.approval.requested`
- `sorx.provider.operation.started`
- `sorx.provider.operation.completed`
- `sorx.endpoint.completed`
- `sorx.endpoint.failed`

## Idempotency

For mutating operations:

- read `Idempotency-Key`
- include key in `EndpointInvocation`
- in-memory provider should return same result for same key and operation
- audit should record `idempotency_key_present: true`

## Tests

Add tests:

- low-risk operation executes
- medium-risk follows config
- high-risk requires approval by default
- critical is denied by default
- auto-approval broker allows high-risk if configured
- deny broker denies operation
- provider is not called when approval required
- provider is not called when denied
- audit events are emitted in expected order
- idempotency prevents duplicate create
- missing risk metadata on mutation fails in strict mode

## Acceptance criteria

- Risk metadata affects runtime execution.
- Approval broker abstraction exists.
- Audit events are structured.
- Provider calls are blocked when policy requires it.
- Idempotency support exists for mutating operations.
- Tests cover policy, approvals, audit, and idempotency.

## Codex working style

Complete as much as possible in one pass. Keep approval implementation local/simple and document future integration with Teams/Slack/web approval flows.
