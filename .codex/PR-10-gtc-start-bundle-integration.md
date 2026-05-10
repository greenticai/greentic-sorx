# PR 10 — `gtc` Extension and Future Bundle/Start Integration

## Goal

Make `greentic-sorx` fit cleanly into the Greentic command and packaging model.

Sorx should be invokable directly as `greentic-sorx` and via `gtc sorx` if the extension mechanism supports `greentic-*` command discovery.

It should also prepare for `.gtbundle` composition without making Sorx assemble final bundles itself.

## Commands

Ensure these work or are documented depending on current `gtc` extension support:

```bash
greentic-sorx start landlord.gtpack --schema
greentic-sorx start landlord.gtpack --answers answers.json

gtc sorx start landlord.gtpack --schema
gtc sorx start landlord.gtpack --answers answers.json
```

## Integration audit

Audit actual `gtc` extension mechanism.

Document:

- how `gtc` discovers external commands
- naming convention
- argument forwarding
- exit code expectations
- JSON output expectations
- install/toolchain requirements

Update `docs/audit/reuse-audit.md`.

## Start compatibility

Sorx should align with `gtc start` answer-driven semantics:

```text
--schema
--answers
--emit-answers
--dry-run
--non-interactive
```

Do not invent a different lifecycle.

## Pack extension metadata

Define or consume a pack extension declaration:

```json
{
  "extension": "greentic.sorx.runtime.v1",
  "sorla": {
    "model": "assets/sorla/model.cbor",
    "agent_gateway": "assets/sorla/agent-gateway.json",
    "mcp_tools": "assets/sorla/mcp-tools.json"
  },
  "sorx": {
    "start_schema": "assets/sorx/start.schema.json",
    "start_questions": "assets/sorx/start.questions.cbor",
    "runtime_template": "assets/sorx/runtime.template.yaml",
    "provider_bindings_template": "assets/sorx/provider-bindings.template.yaml"
  }
}
```

Use existing manifest conventions if different.

## Future `.gtbundle` integration

Do not implement full bundle assembly in Sorx.

Instead, define what a bundle would contain:

```text
app pack:
  landlord-tenant-sor.gtpack

runtime/tool:
  greentic-sorx

provider packs:
  foundationdb provider
  policy/approval provider if available

setup answers:
  sorx startup answers

start metadata:
  Sorx service should be launched with pack and answers
```

Add a design doc:

```text
docs/design/sorx-gtbundle-integration.md
```

Cover:

- how Sorx app packs are included in bundles
- how provider packs are resolved
- how startup answers are supplied
- how service launch could be delegated to `gtc start`
- how future signing/digest checks fit
- how rollback would work by pack version/digest

## Exit codes

Define stable exit codes:

```text
0 success
1 generic error
2 invalid CLI usage
3 pack validation failed
4 answers validation failed
5 provider resolution failed
6 runtime startup failed
7 policy denied during dry-run
```

## Machine-readable output

Add `--json` to relevant commands:

```bash
greentic-sorx doctor landlord.gtpack --json
greentic-sorx inspect landlord.gtpack --json
greentic-sorx routes landlord.gtpack --json
greentic-sorx start landlord.gtpack --answers answers.json --dry-run --json
```

## Tests

Add tests:

- command aliases behave consistently
- `--schema` output compatible with existing Greentic answer style
- `--json` output is stable
- exit codes match error type
- extension-compatible invocation does not require current working dir assumptions
- docs include bundle/start integration guidance

If actual `gtc sorx` integration cannot be tested locally, document why and provide manual test steps.

## Acceptance criteria

- Sorx CLI aligns with `gtc start`.
- Direct `greentic-sorx` invocation works.
- `gtc sorx` integration is implemented if extension mechanism exists.
- Bundle integration is designed without Sorx becoming a bundler.
- Stable exit codes and JSON outputs exist.
- Tests cover CLI compatibility.

## Codex working style

Complete as much as possible in one pass. Audit before assuming `gtc` extension behaviour.
