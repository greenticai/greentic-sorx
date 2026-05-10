# greentic-sorx

CLI crate for the Greentic System of Record eXecutor.

The command surface was scaffolded in PR 01. PR 02 added real `.gtpack`
doctor/inspect behavior, PR 03 added startup schema and answer validation paths,
PR 04 added the shared runtime core, PR 05 added generated route listing and
a local HTTP runtime adapter for `start --answers` / `run --answers`, and PR 06
added policy/approval outcomes to HTTP responses. PR 07 added
`mcp-tools <pack.gtpack>` and `mcp start <pack.gtpack> --answers <file>` for
resolved MCP tool metadata and an adapter-only MCP runtime plan. PR 08 added
provider binding resolution and a FoundationDB adapter boundary; memory remains
the default CI/local provider, while FoundationDB reports a clear unavailable
error until a SORX-compatible store provider is wired.

PR 09 added the landlord/tenant memory-provider e2e scenario. Run it from the
workspace root with `bash scripts/e2e/run-landlord-tenant.sh --provider memory`.

PR 10 added stable exit-code documentation, `--json` compatibility flags for
metadata/dry-run command shapes, and `gtc`/`.gtbundle` integration guidance.
The audited `gtc` implementation does not currently expose a generic `gtc sorx`
passthrough route, so direct `greentic-sorx ...` invocation remains the tested
path.

Full MCP server transport, external approval integrations, and deployment
lifecycle behavior are planned in later PRs.
