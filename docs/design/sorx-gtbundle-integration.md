# SORX `.gtbundle` Integration

SORX consumes `.gtpack` application packs. It should not become a bundle
assembler; bundle composition belongs in Greentic bundle tooling.

## Bundle Contents

A future SORX bundle should contain:

- app pack: the SoRLa/SORX runtime `.gtpack`
- runtime/tool: `greentic-sorx`
- provider packs: FoundationDB and approval/policy providers as needed
- setup answers: SORX startup answers JSON or a managed answer reference
- start metadata: the service launch contract for pack + answers

## App Pack

The app pack remains the runtime handoff boundary. The pack manifest extension
declares:

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

SORX validates and consumes this metadata but does not rewrite the pack.

## Provider Packs

Provider packs are resolved by bundle/start tooling. SORX startup answers refer
to provider configuration by `config_ref`; direct provider `config` is only for
local/test answers.

The FoundationDB provider path remains blocked until a SORX-compatible CRUD
store adapter exists. See `docs/audit/provider-foundationdb-gap-pr08.md`.

## Startup Answers

Bundles should carry either:

- an answers file passed as `greentic-sorx start <pack.gtpack> --answers <file>`
- a managed reference that `gtc start` resolves to such a file before launching
  SORX

Secrets must remain outside `.gtpack` and should be supplied through external
configuration referenced by `config_ref`.

## Launch Delegation

The future `gtc start` integration should delegate to:

```bash
greentic-sorx start app.gtpack --answers sorx.answers.json
```

For validation/planning:

```bash
greentic-sorx start app.gtpack --schema --json
greentic-sorx start app.gtpack --answers sorx.answers.json --dry-run --json
```

## Signing and Digests

Bundle tooling should verify pack signatures and exact digests before launching
SORX. SORX already carries pack digest metadata through route output where
available; future deployment registry work can persist and enforce those
digests per deployment.

## Rollback

Rollback should be a deployment registry concern:

- keep immutable pack name/version/digest records
- keep provider namespace scoped by tenant, pack, and version
- move aliases such as `stable` back to a previous deployment
- restart SORX with the previous pack and answers reference

SORX should execute the selected pack deterministically rather than deciding
which bundle version is active.
