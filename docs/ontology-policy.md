# Ontology Policy

SORX supports ontology-aware policy decisions for concepts, entities, fields,
relationships, evidence, actions, and external references.

Decision shape:

```json
{
  "decision": "deny",
  "reasons": [
    {
      "code": "pii_requires_policy",
      "message": "Tenant.email is marked as sensitive"
    }
  ],
  "redactions": [
    {
      "entity_type": "Tenant",
      "field": "email"
    }
  ]
}
```

The default ontology policy allows reads and traversal. Packs may include static
policy hints under `ontology.graph.json` `policy`, such as:

```json
{
  "policy": {
    "deny_relationships": ["tenant_makes_payment"],
    "sensitive_fields": {
      "Tenant": ["email"]
    },
    "evidence_requires_approval": true
  }
}
```

Graph and evidence commands enforce these hints before expanding relationships
or retrieving evidence.
