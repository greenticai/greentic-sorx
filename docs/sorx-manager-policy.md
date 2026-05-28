# Sorx Manager Policy

Manager filtering is a render-time UX layer. It can hide records, fields, relationships, and actions, or mark them as read-only, redacted, or approval-required. It is not the security boundary.

Submit-time enforcement remains in the normal SORX runtime path through `SorxRuntime::invoke`. That path applies risk policy, approval brokers, provider bindings, idempotency, and audit events.

## Current Model

Core manager policy types live in `greentic-sorx-core`:

```text
ManagerPolicyEffect
ManagerPolicyDecision
ManagerPolicySet
filter_manager_view
```

Manager action state is generated from existing endpoint metadata and `PolicyEngine` decisions. Ontology-aware policy types such as `OntologyPolicyDecision` and `SensitivityContext` remain the source for future deeper field/relationship decisions.

## Non-Goals

- No new policy language.
- No duplicate approval engine.
- No card-submit bypass.
- No domain-specific manager policy names.
