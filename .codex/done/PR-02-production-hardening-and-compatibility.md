# PR 02 - SORX production hardening and compatibility gates

## Repository

- `greenticai/greentic-sorx`

## Objective

Harden the SORX ontology/runtime path for production use.

This PR should not add new features first. It should make SORX-owned ontology, provider compatibility, startup, evidence, policy, audit, validation, deployment, and CLI output contracts safe, versioned, testable, and operable.

Artifacts produced by other repositories are inputs to SORX. This repo should validate and consume those inputs; it should not define implementation work for their producers.

## Hardening areas

### 1. Schema versioning

SORX-owned outputs must have explicit schema names and compatibility tests. Relevant schemas include:

```text
greentic.sorx.start.schema.v1
greentic.sorx.start.answers.normalized.v1
greentic.sorx.start.plan.v1
greentic.sorx.graph.concepts.v1
greentic.sorx.graph.relationships.v1
greentic.sorx.graph.paths.v1
greentic.sorx.graph.neighbors.v1
greentic.sorx.graph.explain.v1
greentic.sorx.evidence-query-result.v1
greentic.sorx.ontology-policy-decision.v1
greentic.sorx.validation-suite.v1
greentic.sorx.validation-report.v1
greentic.sorx.deployment-registry.v1
greentic.sorx.public-routes.v1
```

SORX should also keep explicit allowlists for external input schemas it consumes, such as ontology graph, retrieval bindings, agent gateway, MCP tools, and provider capability metadata.

### 2. Compatibility rules

Define SORX compatibility behavior for pack inputs and runtime outputs:

- known-compatible schema versions
- reject unknown major versions
- ignore or preserve additive input fields on known major versions where safe
- emit stable compatibility errors
- include provider compatibility status in doctor, start dry-run, evidence query, validation, and public-exposure gates where applicable

### 3. Determinism

Verify SORX-owned deterministic behavior:

- stable CLI JSON output
- stable graph traversal order
- stable evidence query planning and deterministic evidence fixtures
- stable doctor and validation report issue ordering
- stable provider compatibility issue ordering
- stable startup plan output

### 4. Security

Add or verify SORX checks for:

- no secrets in consumed artifacts
- no inline credential-like values in startup answers
- PII/sensitivity preserved
- audit redaction
- no public exposure without validation gates
- policy denial for restricted ontology concepts and sensitive evidence
- stable, non-sensitive explain output

### 5. CI

Add/extend local checks:

```bash
bash ci/local_check.sh
```

Ensure this repo checks:

- fmt
- clippy
- tests
- docs
- packaging or release dry run where applicable
- ontology fixture doctor/start/graph/evidence/validate

### 6. Documentation

Create or update SORX production docs:

- `docs/ontology-production-readiness.md`
- `docs/ontology-security.md`
- `docs/ontology-compatibility.md`

## Acceptance criteria

This repo must pass local checks, and the SORX-only ontology smoke scenario must run deterministically against a checked-in or generated fixture pack.

Required proof points:

- unsupported schema major versions fail with stable errors
- known additive fields are handled according to documented compatibility policy
- startup answer secret checks reject credential-like values
- provider compatibility failures block evidence queries and public exposure where applicable
- public route promotion requires validation gates
- docs describe SORX responsibilities without assigning work to other repositories
