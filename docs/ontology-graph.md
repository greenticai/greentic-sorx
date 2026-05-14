# Ontology Graph

SORX can inspect and traverse static ontology graphs embedded in SoRLa packs.

Supported source assets:

```text
assets/sorla/ontology.graph.json
assets/sorla/retrieval-bindings.json
```

Commands:

```bash
greentic-sorx graph concepts landlord.gtpack --json
greentic-sorx graph relationships landlord.gtpack --json
greentic-sorx graph paths landlord.gtpack --from Tenant --to Payment --json
greentic-sorx graph neighbors landlord.gtpack --entity-type Tenant --entity-id tenant-1 --depth 2 --json
greentic-sorx graph explain landlord.gtpack --from Tenant --to Payment --json
```

Traversal is deterministic:

- concepts are sorted by ID
- relationships are sorted by ID
- paths are sorted by relationship sequence
- traversal depth is bounded
- cycles are skipped

PR 03 traverses the static type graph only. Provider-backed relationship
instances are not required for these commands.
