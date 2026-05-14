# PR 07 — MCP Tool Runtime from `mcp-tools.json`

## Goal

Expose MCP tools declared by the SoRLa `.gtpack` and route them through the same internal execution path as HTTP endpoints.

Do not duplicate business logic for MCP.

## Input

Load:

```text
assets/sorla/mcp-tools.json
```

The MCP metadata should map tools to endpoint or operation IDs.

Example:

```json
{
  "schema": "greentic.sorla.mcp-tools.v1",
  "tools": [
    {
      "name": "sorla_create_tenant",
      "description": "Create a tenant record",
      "endpoint_id": "tenant.create",
      "input_schema": {}
    }
  ]
}
```

Use the actual schema if already defined by `greentic-sorla`.

## CLI

Add:

```bash
greentic-sorx mcp-tools landlord.gtpack
greentic-sorx mcp start landlord.gtpack --answers sorx.answers.json
```

If the main server can expose MCP alongside HTTP:

```bash
greentic-sorx start landlord.gtpack --answers sorx.answers.json
```

should honour:

```json
{
  "mcp": {
    "enabled": true,
    "bind": "127.0.0.1:8790"
  }
}
```

## Runtime

MCP invocation lifecycle:

```text
MCP tool call
  → tool lookup
  → endpoint_id / operation_id resolution
  → input validation
  → EndpointInvocation
  → PolicyEngine
  → ApprovalBroker
  → ProviderRegistry
  → AuditSink
  → MCP response
```

Same route as HTTP.

## Tool listing

Implement:

```bash
greentic-sorx mcp-tools landlord.gtpack
```

Output:

```json
{
  "schema": "greentic.sorx.mcp-tools.v1",
  "tools": [
    {
      "name": "sorla_create_tenant",
      "endpoint_id": "tenant.create",
      "operation_id": "tenant.create",
      "risk": "medium"
    }
  ]
}
```

Also expose over HTTP:

```text
GET /v1/sorx/tools
```

## Validation

Extend `doctor` to validate:

- `mcp-tools.json` parses
- tool names are unique
- each tool maps to a valid endpoint or operation
- input schemas are valid
- risk metadata is resolvable
- mutating MCP tools are subject to policy

## Tests

Add tests:

- MCP tools load from pack
- invalid tool reference fails doctor
- duplicate tool name fails doctor
- MCP tool list is stable
- MCP create tenant calls same endpoint router
- high-risk MCP tool requires approval
- audit records MCP as channel/source
- MCP and HTTP produce equivalent results for same operation

If a full MCP server implementation is too heavy, implement the adapter and tool invocation tests first, then document server transport follow-up. But prefer a real local MCP server if existing Greentic MCP conventions exist.

## Acceptance criteria

- Sorx can list MCP tools from `.gtpack`.
- MCP tool calls route through the same execution path as HTTP.
- Policy/approval/audit applies to MCP.
- Doctor validates MCP metadata.
- Tests cover tool loading and invocation.

## Codex working style

Complete as much as possible in one pass. Reuse existing MCP crates/conventions if available. Do not create a parallel business-logic path for MCP.
