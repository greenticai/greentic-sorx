# MCP

SORX reads MCP tool metadata from `assets/sorla/mcp-tools.json` in the pack.

```bash
greentic-sorx mcp-tools landlord.gtpack
greentic-sorx mcp start landlord.gtpack --answers landlord.answers.json
```

`mcp-tools` emits the resolved `greentic.sorx.mcp-tools.v1` list. Tool entries
must reference known endpoint IDs or operation IDs from `agent-gateway.json`.

The core `McpRuntime` adapter invokes tools through the same runtime path as
direct and HTTP invocations: routing, input validation, policy, approvals,
provider bindings, idempotency, and audit. Full MCP server transport is still a
planned integration step.
