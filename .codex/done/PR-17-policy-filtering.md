# PR-02 — Add Policy-Aware Manager View Filtering

## Goal

Add the policy decision model and filtering pipeline used by Sorx Business Manager before rendering any card or manager view.

## Principle

Render-time filtering improves UX. Submit-time enforcement provides security.

Cards should only show records, fields, relationships, and actions that the actor is allowed to see or request, but every submitted action must still re-evaluate policy inside the normal SORX runtime path.

## Add policy decision types

Suggested location:

```text
crates/greentic-sorx-core/src/manager/
  policy.rs
  view_model.rs
  filter.rs
```

Current codebase alignment:

- `crates/greentic-sorx-core/src/policy.rs` already defines `PolicyEngine`, risk-based `PolicyDecision`, and ontology-aware `OntologyPolicyDecision`/`SensitivityContext` with field redaction metadata.
- Manager policy types should adapt existing `PolicyAction`, `OntologyPolicyDecisionKind`, and `OntologyPolicyRedaction` into render-time view effects. Do not fork a second enforcement policy engine.
- Business actions already exist through `assets/sorla/business-actions.json` and `BusinessActionAssets`; manager action filtering should use those assets plus endpoint policy decisions instead of inventing a separate action source.
- Submit-time enforcement is already centralized in `SorxRuntime::invoke`; manager submit handling must delegate there rather than duplicating provider, idempotency, approval, or audit logic.

### `ManagerPolicyDecision`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagerPolicyEffect {
    Allow,
    ReadOnly,
    Redact,
    Hide,
    RequiresApproval,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagerPolicyDecision {
    pub effect: ManagerPolicyEffect,
    pub reason_code: Option<String>,
    pub message_key: Option<String>,
    pub audit_hint: Option<String>,
}
```

## Manager view model

Add a canonical, channel-neutral manager view model.

The base view should be generated from the runtime's existing loaded metadata:

- `EndpointRouter` / `EndpointDefinition` for routes, operations, risk, approval, entity, collection, and input/output schemas.
- Optional pack assets: `ontology.graph.json`, `business-actions.json`, `metrics.json`, `operational-indexes`, and validation-suite fixtures where present.
- Provider bindings only for scoping and provider capability hints; view generation must not call providers unless a route explicitly queries records/pickers.

```rust
pub struct ManagerViewModel {
    pub schema: String, // greentic.sorx.manager-view.v1
    pub tenant_id: String,
    pub sor_id: String,
    pub locale: String,
    pub navigation: Vec<ManagerNavItem>,
    pub records: Vec<ManagerRecordView>,
    pub relationships: Vec<ManagerRelationshipView>,
    pub actions: Vec<ManagerActionView>,
    pub policies: Vec<ManagerPolicyHint>,
}
```

Records, fields, and actions should include policy state after filtering.

## Filtering rules

The filter should support:

- record hidden entirely
- field hidden
- field redacted
- field read-only
- action hidden
- action shown as requiring approval
- relationship hidden when either side is not visible
- relationship degraded when one side is visible only as limited context

## Required tests

Use generic fixtures only.

Test cases:

1. User can view record but not edit fields.
2. User can view redacted sensitive field.
3. User cannot see denied record.
4. User sees action as approval-required.
5. Relationship is hidden when target record is hidden.
6. Submit payload never bypasses policy re-check.

## Non-goals

- Do not implement a new policy language.
- Reuse existing SORX policy/runtime hooks and ontology policy types.
- Do not duplicate the existing approval/idempotency/audit behavior in manager code.
- Do not hardcode landlord/tenant, finance, maintenance, or any other domain.

## Acceptance criteria

- `ManagerViewModel` can be generated and filtered without channel-specific rendering.
- Filtered view is deterministic for the same context and policy inputs.
- Tests prove record/field/action/relationship filtering.
