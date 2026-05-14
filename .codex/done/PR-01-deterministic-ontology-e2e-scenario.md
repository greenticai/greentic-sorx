# PR 01 - Deterministic ontology-driven SORX scenario

## Repository

- `greenticai/greentic-sorx`

## Objective

Create a deterministic SORX-owned scenario proving that an ontology-enabled `.gtpack` can be validated, dry-run started, traversed, queried for evidence, and explained with stable policy and audit output.

The `.gtpack` itself is an input fixture for this repo. Pack authoring and provider catalog generation belong to other repositories and are intentionally out of scope here.

## Scenario requirements

Add or update a generic business-domain fixture under the SORX test fixtures or examples. Domain-specific names are allowed in fixture data, but SORX contracts should continue to use generic ontology concepts and runtime fields.

Recommended fixture:

```text
Party
Customer
Supplier
Contract
Asset
Obligation
EvidenceDocument
```

Relationships:

```text
Customer has_contract Contract
Supplier fulfils_obligation Obligation
Contract governs Asset
Contract has_evidence EvidenceDocument
```

Actions:

```text
CreateCustomer
AttachEvidenceToContract
ListContractsForCustomer
AssessObligationRisk
```

## Flow

1. SORX loads a fixture `.gtpack` containing ontology graph and retrieval binding assets.
2. `greentic-sorx doctor --json` validates required pack, ontology, retrieval, startup schema, provider compatibility, and validation-suite assets.
3. `greentic-sorx start --dry-run --json` validates startup answers and emits a stable startup plan.
4. `greentic-sorx graph paths --json` finds the path from `Customer` to `EvidenceDocument`.
5. `greentic-sorx evidence query --json` retrieves deterministic ontology-scoped evidence.
6. SORX policy checks allow or deny evidence access based on ontology sensitivity and policy hints.
7. SORX explain and audit output records concepts used, graph paths considered, evidence IDs, policy decisions, and redactions.

## Required scripts

Add one top-level script or test entry point in this repo:

```bash
scripts/e2e/ontology-smoke.sh
```

It should be CI-safe and deterministic.

## Acceptance criteria

The final command sequence should be SORX-only and look like:

```bash
greentic-sorx doctor examples/ontology-business/ontology-business.gtpack --json
greentic-sorx start examples/ontology-business/ontology-business.gtpack --answers examples/ontology-business/sorx.answers.json --dry-run --json
greentic-sorx graph paths examples/ontology-business/ontology-business.gtpack --from Customer --to EvidenceDocument --json
greentic-sorx evidence query examples/ontology-business/ontology-business.gtpack --answers examples/ontology-business/sorx.answers.json --query "risk evidence" --entity-type Customer --entity-id customer-001 --json
```

Expected results:

- doctor exits successfully and emits stable JSON
- start dry-run emits `greentic.sorx.start.plan.v1`
- graph paths emits `greentic.sorx.graph.paths.v1`
- evidence query emits `greentic.sorx.evidence-query-result.v1`
- repeated runs produce deterministic machine-readable output after removing timestamps, if any
- tests cover the fixture path in CI-safe form without needing external services

## Docs

Add or update a SORX guide:

```text
docs/ontology-driven-sorx-scenario.md
```

The guide should explain how SORX consumes the fixture pack and answers. It should not document pack authoring steps as work owned by this repo.
