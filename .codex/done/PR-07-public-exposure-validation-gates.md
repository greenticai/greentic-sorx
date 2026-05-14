# PR 07 — Add public exposure gates for ontology-enabled packs

## Repository

`greenticai/greentic-sorx`

## Objective

Ensure ontology-enabled packs cannot be promoted to public exposure unless required validation suites pass or an explicit local operator policy override exists.

## Gated areas

Public exposure must check:

- pack doctor passed
- ontology static validation passed
- provider compatibility passed
- retrieval binding validation passed
- policy validation passed
- high-risk endpoint approval requirements are satisfied
- no secret-like values in runtime answers
- public routes are explicitly allowed by exposure policy

## Commands to update

```bash
greentic-sorx deployments validate <deployment-id>
greentic-sorx deployments promote <deployment-id> --public
greentic-sorx deployments promotion-status <deployment-id>
```

## Promotion status output

```json
{
  "deployment_id": "...",
  "public_eligible": false,
  "gates": [
    {
      "id": "ontology-static",
      "status": "passed"
    },
    {
      "id": "provider-compatibility",
      "status": "failed",
      "reason": "missing entity-link provider"
    }
  ]
}
```

## Tests

Add tests for:

- public promotion denied when ontology validation missing
- public promotion denied when provider compatibility fails
- private activation still allowed when configured
- explicit override is recorded and audited
- promotion status JSON stable

## Docs

Update:

- `docs/deployments.md`
- `docs/security.md`
- `docs/validation-suites.md`

## Acceptance criteria

```bash
cargo test --all-features
bash ci/local_check.sh
```
