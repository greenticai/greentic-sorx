# PR 02 — `.gtpack` Loader, Inspector, and Sorx Doctor

## Goal

Implement real `.gtpack` loading and validation for SoRLa/Sorx runtime packs.

Sorx must consume `.gtpack` as its primary input. Do not add support for loose folders except perhaps an internal test helper.

## Expected pack contract

A SoRLa executable pack should contain:

```text
pack.cbor
pack.lock.cbor

assets/sorla/model.cbor
assets/sorla/agent-gateway.json
assets/sorla/agent-endpoints.openapi.overlay.yaml
assets/sorla/agent-workflows.arazzo.yaml
assets/sorla/mcp-tools.json
assets/sorla/llms.txt.fragment

assets/sorx/start.schema.json
assets/sorx/start.questions.cbor
assets/sorx/runtime.template.yaml
assets/sorx/provider-bindings.template.yaml
```

Some assets may be optional depending on manifest metadata, but `model.cbor`, `agent-gateway.json`, and `start.schema.json` should be required for a runtime pack.

## Reuse existing pack code

Before implementing archive/CBOR parsing, check whether `greentic-pack` exposes any library or CLI helpers for:

- reading `.gtpack`
- listing assets
- reading `pack.cbor`
- reading `pack.lock.cbor`
- validating locks/digests
- resolving pack extension metadata
- doctor/inspect output

Reuse existing APIs if available. If not available, implement a minimal local loader and document why in `docs/audit/reuse-audit.md`.

## Add crates/modules

Suggested modules:

```text
crates/greentic-sorx-pack/
  src/lib.rs
  src/loader.rs
  src/doctor.rs
  src/inspect.rs
  src/manifest.rs
```

Only create a separate crate if consistent with repo style. Otherwise keep modules under `greentic-sorx-core`.

## Core data structures

Add:

```rust
pub struct LoadedSorlaPack {
    pub pack_path: PathBuf,
    pub pack_name: String,
    pub pack_version: String,
    pub pack_digest: Option<String>,
    pub manifest: PackManifest,
    pub lock: Option<PackLock>,
    pub sorla_assets: SorlaAssets,
    pub sorx_assets: SorxAssets,
}

pub struct SorlaAssets {
    pub model_cbor: Vec<u8>,
    pub agent_gateway_json: serde_json::Value,
    pub openapi_overlay_yaml: Option<String>,
    pub arazzo_yaml: Option<String>,
    pub mcp_tools_json: Option<serde_json::Value>,
    pub llms_txt_fragment: Option<String>,
}

pub struct SorxAssets {
    pub start_schema_json: serde_json::Value,
    pub start_questions_cbor: Option<Vec<u8>>,
    pub runtime_template_yaml: Option<String>,
    pub provider_bindings_template_yaml: Option<String>,
}

pub struct SorxDoctorReport {
    pub ok: bool,
    pub errors: Vec<SorxDoctorIssue>,
    pub warnings: Vec<SorxDoctorIssue>,
}
```

Use existing manifest/lock types if they exist.

## Doctor validation rules

Implement:

```bash
greentic-sorx doctor landlord.gtpack
```

Checks:

- input path exists
- extension is `.gtpack`
- pack opens successfully
- `pack.cbor` exists
- `pack.lock.cbor` exists or warning if current Greentic format allows missing lock
- Sorx runtime extension metadata exists
- referenced SorLa assets exist
- referenced Sorx startup assets exist
- no manifest reference points outside allowed asset paths
- `model.cbor` can be read
- `agent-gateway.json` parses
- `start.schema.json` parses
- `mcp-tools.json` parses if present
- `agent-endpoints.openapi.overlay.yaml` parses as YAML if present
- `agent-workflows.arazzo.yaml` parses as YAML if present
- no asset path traversal
- no obvious secret-like values are embedded in runtime templates or startup answers

Secret scanning can be conservative and warning-only initially.

## Inspect command

Implement:

```bash
greentic-sorx inspect landlord.gtpack
```

Output stable JSON by default or with `--json`.

Include:

```json
{
  "schema": "greentic.sorx.inspect.v1",
  "pack": {
    "name": "landlord-tenant-sor",
    "version": "0.1.0",
    "digest": "..."
  },
  "sorla": {
    "has_model": true,
    "has_agent_gateway": true,
    "has_mcp_tools": true
  },
  "sorx": {
    "has_start_schema": true,
    "has_runtime_template": true
  }
}
```

## Tests

Add fixtures:

```text
tests/fixtures/valid-landlord.gtpack
tests/fixtures/missing-model.gtpack
tests/fixtures/invalid-gateway.gtpack
tests/fixtures/missing-start-schema.gtpack
```

If producing real `.gtpack` fixtures is too heavy, create them programmatically in tests using existing pack APIs.

Test:

- valid pack passes doctor
- missing required assets fail doctor
- invalid JSON fails doctor
- invalid YAML warns or fails as appropriate
- inspect output is stable
- path traversal references fail
- missing optional MCP tools is allowed if manifest marks them optional

## Acceptance criteria

- `doctor` performs real pack validation.
- `inspect` emits stable JSON.
- Required assets are enforced.
- Runtime input is `.gtpack`.
- Existing `greentic-pack` code is reused where available.
- Tests cover valid and invalid packs.

## Codex working style

Complete as much as possible in one pass. If existing pack APIs are missing, implement a minimal local loader and document the gap.


## v2 additions — validation-suite awareness

Extend the loader model so it preserves optional SORX validation-suite assets even before PR 14 implements execution. The loader must not fail if these files are absent in legacy packs, but it must expose them in `pack inspect` when present:

```text
assets/sorx/validation-suite.cbor
assets/sorx/validation-suite.json
assets/sorx/validation-fixtures/**
assets/sorx/validation-openapi.expected.json
```

Doctor should classify validation-suite status as:

```text
missing       legacy pack or no public exposure requested
present       suite manifest exists and references valid files
invalid       manifest exists but schema/references are broken
```

Do not execute the suite in PR 02. Execution and public-gate semantics belong to PR 14 and PR 15.
