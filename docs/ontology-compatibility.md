# Ontology Compatibility

SORX owns compatibility behavior for the runtime contracts it emits and the
pack inputs it consumes.

SORX-owned output schemas include:

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
greentic.sorx.validation-suite.v1
greentic.sorx.validation-report.v1
greentic.sorx.deployment-registry.v1
greentic.sorx.public-routes.v1
```

SORX currently consumes these ontology input schemas:

```text
greentic.sorla.ontology.graph.v1
greentic.sorla.retrieval-bindings.v1
```

Compatibility policy:

- unknown ontology or retrieval-binding schema major versions fail validation
  with stable doctor errors
- additive fields on known input schemas are preserved in parsed asset metadata
  and ignored by SORX validation unless they are unsafe
- provider contract versions must be compatible with `greentic.sorx.provider.v1`
- provider compatibility issues are sorted by requirement, category, and message
- provider capability lists are sorted and deduplicated before startup-plan and
  compatibility output
