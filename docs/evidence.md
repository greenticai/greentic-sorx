# Evidence

`greentic-sorx evidence query` runs a deterministic ontology-scoped evidence
query against a configured provider capability.

```bash
greentic-sorx evidence query landlord.gtpack \
  --answers landlord.answers.json \
  --query "lease status" \
  --entity-type Tenant \
  --entity-id tenant-1 \
  --max-depth 2 \
  --json
```

The command:

- loads the pack ontology graph
- validates the requested entity type
- builds an ontology scope from nearby static graph relationships
- requires a provider with `ontology-scoped-evidence-query`
- returns evidence with linked entities, source refs, provenance, and explain data
- includes deterministic audit events for provider compatibility, policy
  decisions, planning, entity linking, and evidence execution
- emits explain fields for ontology graph hash, concepts, relationships,
  providers, evidence IDs, policy decisions, and redactions

PR 04 uses a deterministic local evidence provider path for tests and dry local
execution. It does not call an LLM.
