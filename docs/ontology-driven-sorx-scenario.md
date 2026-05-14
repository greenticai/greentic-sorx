# Ontology-Driven SORX Scenario

This scenario proves the SORX-owned part of the ontology runtime path with a
deterministic business-domain fixture. The pack is treated as an input to this
repository; authoring that pack belongs outside SORX.

The executable coverage lives in the CLI smoke test:

```bash
cargo test -p greentic-sorx-cli binary_deterministic_ontology_business_scenario_is_stable
```

The fixture model uses generic ontology concepts:

```text
Party
Customer
Supplier
Contract
Asset
Obligation
EvidenceDocument
```

The scenario validates these SORX behaviors:

- `doctor --json` accepts the ontology-enabled pack
- `start --dry-run --json` emits a stable startup plan and provider compatibility result
- `graph paths --from Customer --to EvidenceDocument --json` finds a deterministic path
- `evidence query --entity-type Customer --entity-id customer-001 --json` returns deterministic evidence
- explain output records graph paths, concepts, relationships, providers, evidence IDs, policy decisions, and redactions
- audit events include provider compatibility, ontology policy, query planning, entity linking, and evidence execution

The graph includes a traversable evidence edge from `Contract` to
`EvidenceDocument` so SORX can prove the `Customer -> EvidenceDocument` path
using its directed static traversal.
