# PR 04 — Runtime Core, Operation Router, and Provider Traits

## Goal

Create the internal Sorx runtime core that maps SoRLa endpoint invocations to provider-backed system-of-record operations.

This PR should not yet expose a full HTTP server. It should create the core execution path that HTTP and MCP will both use.

## Core idea

HTTP and MCP must share the same execution path:

```text
HTTP route ┐
           ├── EndpointRouter → Policy → Provider operation → Audit → Response
MCP tool  ┘
```

This PR builds `EndpointRouter`, provider traits, and in-memory provider.

## Modules

Suggested:

```text
crates/greentic-sorx-core/
  src/runtime.rs
  src/router.rs
  src/invocation.rs
  src/provider.rs
  src/providers/memory.rs
  src/model.rs
  src/error.rs
```

## Data structures

Add:

```rust
pub struct SorxRuntime {
    pub pack: LoadedSorlaPack,
    pub config: SorxRuntimeConfig,
    pub router: EndpointRouter,
    pub providers: ProviderRegistry,
}

pub struct EndpointRouter {
    pub endpoints: HashMap<String, EndpointDefinition>,
}

pub struct EndpointDefinition {
    pub endpoint_id: String,
    pub operation_id: String,
    pub method: EndpointMethod,
    pub path: String,
    pub entity: Option<String>,
    pub risk: RiskLevel,
    pub input_schema: Option<serde_json::Value>,
    pub output_schema: Option<serde_json::Value>,
}

pub struct EndpointInvocation {
    pub tenant_id: String,
    pub endpoint_id: String,
    pub operation_id: String,
    pub input: serde_json::Value,
    pub caller: CallerContext,
    pub idempotency_key: Option<String>,
}

pub struct EndpointResult {
    pub status: EndpointStatus,
    pub output: serde_json::Value,
    pub events: Vec<SorxEvent>,
}
```

## Provider traits

Define local traits first. Later they can be backed by `greentic-sorla-providers`.

```rust
#[async_trait]
pub trait SorStoreProvider: Send + Sync {
    async fn create(&self, op: CreateOp) -> SorxResult<EntityRecord>;
    async fn get(&self, op: GetOp) -> SorxResult<Option<EntityRecord>>;
    async fn update(&self, op: UpdateOp) -> SorxResult<EntityRecord>;
    async fn query(&self, op: QueryOp) -> SorxResult<QueryResult>;
    async fn delete(&self, op: DeleteOp) -> SorxResult<DeleteResult>;
}
```

Add:

```rust
pub struct ProviderRegistry {
    stores: HashMap<String, Arc<dyn SorStoreProvider>>,
}
```

## Operation mapping

Implement a minimal mapping from endpoint/operation metadata to provider operations.

For example:

```text
tenant.create → CreateOp { entity: "Tenant", collection: "tenants" }
tenant.get    → GetOp { entity: "Tenant", id: ... }
tenant.update → UpdateOp { entity: "Tenant", id: ..., patch: ... }
tenant.query  → QueryOp { entity: "Tenant", filter: ... }
```

Do not hard-code landlord-specific operations except in fixtures/tests.

Use gateway metadata where possible.

## In-memory provider

Implement deterministic in-memory provider for local e2e:

- entity collections
- create
- get
- update/patch
- query by simple equality filters
- delete
- idempotency support if idempotency key is supplied
- stable IDs where supplied by caller

## Input validation

If endpoint metadata includes JSON schema, validate input before provider execution.

If no schema exists, allow but warn or record a doctor warning.

## Tests

Create landlord-style fixture metadata and test:

- endpoint router builds from gateway metadata
- create tenant operation
- get tenant operation
- update tenant operation
- query active tenants
- missing provider binding fails clearly
- unknown endpoint fails clearly
- invalid input fails before provider execution
- in-memory provider is deterministic
- idempotency key prevents duplicate create where supported

## Acceptance criteria

- Runtime core can execute endpoint invocations without HTTP/MCP.
- Provider trait exists.
- In-memory provider exists.
- Router maps endpoint metadata to operations.
- Tests prove create/get/update/query.
- No HTTP server yet required.
- No FoundationDB yet required.

## Codex working style

Complete as much as possible in one pass. Keep abstractions small and avoid overengineering. Use local traits if external provider APIs do not exist yet.
