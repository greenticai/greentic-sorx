# PR 05 — Add ontology-aware policy and permission checks

## Repository

`greenticai/greentic-sorx`

## Objective

Extend Sorx policy checks so access decisions can be made at ontology concept, relationship, field, evidence, and action levels.

## Current repo alignment

The current policy engine in `crates/greentic-sorx-core/src/policy.rs` is
endpoint-risk based and returns `execute`, `require_approval`, or `deny`.
The runtime already applies it before provider execution and emits audit events.

Extend that model instead of replacing it. Keep existing endpoint risk policy
behavior stable, then add ontology/resource-level decisions where graph,
evidence, external-reference, MCP, and public exposure flows need them.

## Policy dimensions

Support generic checks for:

```text
concept read
entity instance read
field read
relationship traversal
evidence retrieval
agent endpoint invocation
side-effectful action execution
external reference resolution
```

## New types

Add:

- `OntologyPolicySubject`
- `OntologyPolicyResource`
- `OntologyPolicyAction`
- `OntologyPolicyDecision`
- `OntologyPolicyReason`
- `SensitivityContext`
- `RelationshipTraversalPermission`

## Policy decision shape

```json
{
  "decision": "allow | deny | requires_approval",
  "reasons": [
    {
      "code": "pii_requires_policy",
      "message": "Customer.email is marked as PII"
    }
  ],
  "redactions": [
    {
      "entity_type": "Customer",
      "field": "email"
    }
  ]
}
```

## Integration points

Apply checks before:

1. graph traversal with relationship expansion
2. evidence query
3. external reference resolution
4. agent endpoint action execution
5. MCP tool execution
6. public route exposure and existing deployment promotion gates

## Tests

Add tests for:

- concept read allowed
- field read denied because sensitivity
- relationship traversal denied
- evidence query denied
- action requires approval
- redaction list emitted
- audit event emitted for deny/approval
- existing endpoint risk policy tests continue to pass unchanged
- deployment promotion-status output remains backwards compatible unless PR 07 explicitly changes it

## Docs

Add:

- `docs/ontology-policy.md`
- update `docs/security.md`

## Acceptance criteria

```bash
cargo test --all-features
bash ci/local_check.sh
```
