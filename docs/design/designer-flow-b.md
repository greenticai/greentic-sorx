# Designer Flow B

Flow B is the future generated-artifact handoff from Designer into SORX:

```text
Designer session
  -> BundleExtension render
  -> Sorla .gtpack
  -> Sorx artifact validation / inspect / startup schema
  -> future .gtbundle or deployment artifact
```

SORX does not render Designer sessions. It validates the generated `.gtpack`
artifact that a bundle extension produces, then emits stable metadata for the
next step in the pipeline.

## Extension Roles

`DesignExtension` owns interactive authoring, prompt-to-model workflows, and
session state.

`BundleExtension` owns deterministic rendering of a Designer session into a
SoRLa `.gtpack`. Later flows can render `.gtbundle` or deployment artifacts.

`greentic-sorx` owns runtime validation of the generated artifact:

- accept a `.gtpack` file or Designer generic artifact JSON
- verify artifact kind, media type, base64 payload, and SHA-256
- run pack doctor checks
- emit inspect metadata
- emit the startup schema
- optionally evaluate provider compatibility when startup answers are supplied

## CLI

```bash
greentic-sorx artifact validate --artifact-json generated-artifact.json --json
greentic-sorx artifact validate --artifact-json generated-artifact.json --answers answers.json --json
greentic-sorx artifact inspect --artifact-json generated-artifact.json --json
greentic-sorx artifact startup-schema --artifact-json generated-artifact.json --json
```

Path-based input is also supported:

```bash
greentic-sorx artifact validate --file generated.gtpack --json
```

The stable validation report schema is:

```text
greentic.sorx.artifact.validation-report.v1
```
