# Validation Suites

PR14 adds declarative validation suites embedded in `.gtpack` files.

Supported assets:

```text
assets/sorx/validation-suite.cbor
assets/sorx/validation-suite.json
assets/sorx/validation-fixtures/**
assets/sorx/validation-expected/**
```

`validation-suite.cbor` is preferred when present. The JSON form is useful for
inspection and tests.

Minimal suite shape:

```json
{
  "schema": "greentic.sorx.validation-suite.v1",
  "suite_id": "landlord-basic",
  "gates": {
    "required_for_public_exposure": true,
    "minimum_pass_level": "required"
  },
  "tests": [
    {
      "id": "doctor.pack.valid",
      "kind": "doctor",
      "level": "required"
    }
  ]
}
```

Supported test kinds in PR14:

- `doctor`
- `artifact_exists`
- `artifact_schema`
- `route_generation`
- `provider_contract`
- `endpoint_call`
- `negative_endpoint_call`
- `audit_event_emitted`
- `idempotency`
- `policy_denial`

Run a suite:

```bash
greentic-sorx validate landlord.gtpack --answers landlord.answers.json --provider-mode in-memory --json
```

Provider modes:

- `in-memory`: runs against an ephemeral memory provider.
- `mock`: currently equivalent to `in-memory`.
- `configured`: reserved for real configured providers and fails clearly in the
  local runner until those adapters are available.

Reports use stable shape `greentic.sorx.validation-report.v1`. Required test
failures block public readiness. Recommended failures appear in the report but
do not block public readiness. Informational tests never block.

No validation suite test can run shell commands or arbitrary code from a pack.

Pack doctor validation also checks optional SoRLa ontology assets when present:

- `assets/sorla/ontology.graph.json`
- `assets/sorla/ontology.ir.cbor`
- `assets/sorla/retrieval-bindings.json`

Ontology validation is static. It checks supported schemas, unique concept and
relationship IDs, relationship endpoints, optional record metadata references,
retrieval binding references, ontology IR hashes, secret-like values, and
absolute local paths.

Validation reports for ontology-enabled packs can include an `ontology` object
with gate statuses used by public promotion. These gates are intentionally
public-exposure gates; private activation still follows the base validation
report result and digest checks.

```json
{
  "ontology": {
    "static_validation": "passed",
    "provider_compatibility": "passed",
    "retrieval_bindings": "passed",
    "policy_validation": "passed"
  }
}
```
