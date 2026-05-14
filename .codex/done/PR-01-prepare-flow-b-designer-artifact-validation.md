# PR 01 - Prepare Sorx for Flow B: Designer artifact validation and bundle-render handoff

## Repository

`greenticai/greentic-sorx`

## Objective

Prepare Sorx for the future Flow B:

```text
Designer session
  -> BundleExtension render
  -> Sorla .gtpack
  -> Sorx validation / inspect / startup schema
  -> future .gtbundle or deployment artifact
```

This PR should not implement full bundle rendering. It should provide the validation/inspection service shape needed by Designer and future bundle extensions.

## Add generic artifact input support

Existing path-based commands already cover core pack behavior:

```bash
greentic-sorx doctor generated.gtpack --json
greentic-sorx inspect generated.gtpack --json
greentic-sorx start generated.gtpack --schema --json
```

This PR should add the missing Designer-oriented ingestion and combined report
surface for a `.gtpack` supplied as:

- file path
- bytes
- base64 artifact JSON

## Proposed CLI commands

Add one or both:

```bash
greentic-sorx artifact validate --file generated.gtpack --json
greentic-sorx artifact validate --artifact-json generated-artifact.json --json
greentic-sorx artifact validate --artifact-json generated-artifact.json --answers answers.json --json
greentic-sorx artifact inspect --file generated.gtpack --json
greentic-sorx artifact startup-schema --file generated.gtpack --json
```

If command names differ, keep the intent.

## Generic artifact JSON input

Accept the Designer SDK generic artifact shape:

```json
{
  "kind": "gtpack",
  "filename": "example-sor.gtpack",
  "media_type": "application/vnd.greentic.gtpack",
  "sha256": "...",
  "bytes_base64": "...",
  "metadata_json": {}
}
```

Sorx should:

1. Check kind/media type.
2. Decode bytes.
3. Verify SHA-256.
4. Run pack doctor.
5. Run inspect.
6. Emit startup schema if available.
7. Emit provider compatibility status if answers are supplied.
8. Return stable JSON.

## New library API

Add to Sorx pack/CLI as appropriate. Byte-oriented pack loading and inspection
should live in `greentic-sorx-pack`; CLI/report orchestration should live in
`greentic-sorx-cli`. Avoid adding this to `greentic-sorx-core` unless runtime
config, startup answers, or existing core errors are directly needed.

Shape the API around existing repo types such as `LoadedSorlaPack`,
`SorxDoctorReport`, `SorxInspectReport`, and pack-specific errors:

```rust
pub fn load_sorla_pack_from_bytes(bytes: &[u8])
    -> Result<LoadedSorlaPack, SorxPackError>;

pub fn inspect_gtpack_bytes(bytes: &[u8])
    -> Result<SorxInspectReport, SorxPackError>;

pub fn startup_schema_from_gtpack_bytes(bytes: &[u8])
    -> Result<serde_json::Value, SorxPackError>;
```

Avoid depending directly on `greentic-designer-sdk` if that would create an undesirable dependency. Use a local compatible struct if needed.

## Output shape

```json
{
  "schema": "greentic.sorx.artifact.validation-report.v1",
  "valid": true,
  "artifact": {
    "filename": "example-sor.gtpack",
    "sha256": "..."
  },
  "doctor": {},
  "inspect": {},
  "startup_schema": {},
  "provider_compatibility": null,
  "diagnostics": []
}
```

When `--answers` is supplied, include provider compatibility status in
`provider_compatibility`. When answers are omitted, keep the field `null` so the
JSON shape remains stable.

## Future Flow B preparation doc

Add:

```text
docs/design/designer-flow-b.md
```

Explain:

```text
DesignExtension:
  interactive authoring and prompt-to-model

BundleExtension:
  deterministic render of Designer session into Sorla .gtpack and later .gtbundle

Sorx:
  validates generated artifacts and emits startup/provider/route metadata
```

## Tests

Add tests for:

- validate artifact JSON with correct SHA
- reject hash mismatch
- reject wrong media type
- reject non-gtpack kind
- inspect from bytes
- startup schema from bytes
- provider compatibility from artifact JSON plus answers
- stable JSON result
- malformed base64 rejected

## Acceptance criteria

```bash
cargo test --all-features
bash ci/local_check.sh
```
