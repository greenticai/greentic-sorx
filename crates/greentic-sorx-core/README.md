# greentic-sorx-core

Core types for `greentic-sorx`.

This crate contains the shared SORX scaffold types, PR 03 startup support,
PR 04 runtime core, PR 06 policy/approval/audit support, PR 07 MCP tool
adapter support, and PR 08 provider binding resolution.

Startup support includes schema-driven answer normalization, default
application, validation issue reporting, secret-like answer rejection, runtime
config construction, and deterministic startup plan generation.

Runtime support includes endpoint routing from SoRLa agent-gateway metadata,
local provider traits, provider registration, entity binding resolution,
tenant/pack/version provider namespaces, endpoint invocation models, minimal
input validation, a deterministic in-memory store provider for local execution
and tests, and a FoundationDB adapter boundary that fails clearly until a
SORX-compatible store provider is wired.

Policy support includes risk-based execution decisions, local approval broker
traits/implementations, operation-scoped idempotency keys, structured audit
events, stdout/memory/disabled audit sinks, and strict router validation for
missing mutation risk metadata.

MCP support includes loading resolved tool definitions from
`assets/sorla/mcp-tools.json`, validating tool-to-endpoint references, and
invoking tools through the same `SorxRuntime` path used by direct and HTTP
calls.

Runtime pack loading lives in `greentic-sorx-pack`. HTTP lives in
`greentic-sorx-cli`; full MCP server transport, external approval integrations,
FoundationDB execution, mutating HTTP admin storage/auth, and real GHCR
download behavior are intentionally left for later PRs.
