# PR 06 — Add ontology-aware audit and explainability events

## Repository

`greenticai/greentic-sorx`

## Objective

Emit structured audit and explainability events for ontology graph traversal, evidence retrieval, entity linking, policy decisions, provider resolution, and action execution.

## Event categories

Add audit event kinds:

```text
ontology.graph.loaded
ontology.path.resolved
provider.compatibility.checked
evidence.query.planned
evidence.query.executed
entity.links.resolved
policy.ontology.decision
action.ontology.executed
public.exposure.gated
```

## Explainability output

For runtime commands that use ontology/evidence, include:

```json
{
  "explain": {
    "ontology_graph_hash": "...",
    "concepts_used": [],
    "relationships_used": [],
    "providers_used": [],
    "evidence_used": [],
    "policy_decisions": [],
    "redactions": []
  }
}
```

## Requirements

1. Stable event schema.
2. Deterministic ordering.
3. No secrets in audit payloads.
4. Sensitive values must be redacted.
5. Audit sink remains configurable.
6. Works in dry-run and runtime modes.

## Tests

Add tests for:

- graph traversal emits audit event
- evidence query emits planning and execution events
- policy deny emits audit event
- secrets are redacted
- explain output stable

## Docs

Update:

- `docs/observability.md`
- `docs/security.md`
- `docs/evidence.md`

## Acceptance criteria

```bash
cargo test --all-features
bash ci/local_check.sh
```
