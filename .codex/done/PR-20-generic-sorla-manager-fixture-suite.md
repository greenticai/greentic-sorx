# PR-05 — Add Generic SorLa Manager Fixture Suite

## Goal

Create a generic fixture suite proving Sorx Business Manager works for any SorLa-defined system of record.

This replaces any domain-specific fixture plan. Sorx Business Manager must never be tested or implemented as if it is tied to one SorLa domain.

## Core rule

Do not hardcode landlord/tenant, finance, maintenance, CRM, ticketing, inventory, healthcare, or any other domain into manager implementation or tests.

Domain examples may appear in documentation only as illustrative examples. They must not become route names, special cases, fixture assumptions, policy names, labels, or test-only shortcuts.

## Fixture strategy

Use several deliberately small synthetic SorLa/SORX fixture packs. Each fixture proves one generic capability.

Current codebase alignment:

- There is no root `fixtures/` directory today. Existing end-to-end fixtures live under `crates/greentic-sorx-cli/tests/e2e/fixtures/...`, and pack-embedded validation assets are loaded from `assets/sorx/validation-suite.json` plus referenced fixture JSON files.
- Manager fixtures should either live under `crates/greentic-sorx-cli/tests/e2e/fixtures/manager/...` for CLI HTTP tests, or be generated in tests as `.gtpack` archives with `assets/sorx/validation-suite.json` and `assets/sorx/fixtures/...` entries.
- The current landlord/tenant E2E remains as historical coverage; this PR should avoid adding new landlord/tenant manager fixtures, not delete unrelated existing tests.

Suggested fixture names:

```text
crates/greentic-sorx-cli/tests/e2e/fixtures/manager/basic-records
crates/greentic-sorx-cli/tests/e2e/fixtures/manager/references-and-pickers
crates/greentic-sorx-cli/tests/e2e/fixtures/manager/relationship-graph
crates/greentic-sorx-cli/tests/e2e/fixtures/manager/policy-matrix
crates/greentic-sorx-cli/tests/e2e/fixtures/manager/multi-team-access
crates/greentic-sorx-cli/tests/e2e/fixtures/manager/locale-catalog
crates/greentic-sorx-cli/tests/e2e/fixtures/manager/approval-actions
crates/greentic-sorx-cli/tests/e2e/fixtures/manager/sensitive-fields
```

## Naming style

Use neutral synthetic names:

```text
RecordAlpha
RecordBeta
ActorRecord
CaseRecord
EventRecord
TransactionRecord
DocumentRecord
LocationRecord
```

Use fields such as:

```text
id
name
description
status
created_at
amount
owner_ref
asset_ref
case_ref
email_value
sensitive_note
```

The goal is to test capabilities, not business domains.

## Fixtures

### 1. `basic-records`

Proves:

- records become navigation items
- fields become list/detail/create form metadata
- create card is generated generically
- edit/detail card is generated generically

Assertions:

```text
Given any record with scalar fields
→ manager renders list/detail/create cards
→ no domain-specific labels are required
```

### 2. `references-and-pickers`

Proves:

- reference fields become pickers
- picker metadata identifies target record and display field
- raw IDs are not shown as the preferred business input

Assertions:

```text
Given RecordBeta.owner_ref references RecordAlpha.id
→ create/edit card renders owner_ref as a picker
→ picker endpoint scopes results by tenant/team/context
```

### 3. `relationship-graph`

Proves:

- graph nodes are generated from records/concepts
- graph edges are generated from references/relationships
- relationship summary cards render drill-down actions
- SVG is optional and not required for management

Assertions:

```text
Given references and relationship metadata
→ /v1/sorx/manager/graph.json contains generic nodes and edges
→ /v1/sorx/manager/cards/relationships contains textual/drill-down relationship summary
```

### 4. `policy-matrix`

Proves:

- record-level allow/deny
- field-level hide/redact/readonly
- action-level allow/deny/requires-approval
- relationship filtering

Assertions:

```text
Given user/team policy matrix
→ denied records are absent
→ redacted fields do not reveal values
→ readonly fields render as readonly
→ approval-required actions are marked correctly
```

### 5. `multi-team-access`

Proves:

- same generic SorLa pack produces different dashboards for different teams
- team context affects navigation, actions, and visible fields

Use synthetic teams:

```text
team-alpha
team-beta
team-gamma
```

Assertions:

```text
Given the same pack and different team_id values
→ manager renders different allowed views
→ hidden capabilities are not present in generated cards
```

### 6. `locale-catalog`

Proves:

- labels resolve from locale keys
- fallback humanization works
- RTL direction metadata/hints are propagated

Suggested locales:

```text
en-GB
fr-FR
es-ES
ar
```

Assertions:

```text
Given locale fr-FR
→ record/field/action labels use French catalog values
Given missing key
→ fallback label is humanized and deterministic
Given RTL locale
→ view/card includes direction hint where supported
```

### 7. `approval-actions`

Proves:

- generic actions can require approval
- approval requirement is represented in cards
- submit path re-checks policy

Assertions:

```text
Given action X requires approval
→ rendered card shows approval-required state
→ submit does not execute directly without policy approval path
```

### 8. `sensitive-fields`

Proves:

- sensitive fields are treated generically
- policy controls redaction/hiding/editability
- audit hints are available for sensitive access

Assertions:

```text
Given sensitive field sensitive_note
→ unauthorized actor receives hidden/redacted output
→ authorized actor can see allowed representation
→ action/card metadata contains audit hint where applicable
```

## Test categories

Add tests around capabilities, not domains:

```text
manager_generates_navigation_for_any_record
manager_renders_create_card_for_scalar_fields
manager_renders_reference_as_picker
manager_generates_graph_from_references
manager_filters_records_by_policy
manager_redacts_sensitive_fields
manager_varies_view_by_team
manager_localizes_labels
manager_falls_back_to_humanized_labels
manager_rechecks_policy_on_submit
```

## Acceptance criteria

- All fixtures are domain-neutral.
- No new landlord/tenant-specific manager fixture is added in this PR plan.
- Tests assert generic manager behavior.
- Fixtures are small and deterministic.
- Fixtures follow current `.gtpack` required entries: `pack.cbor`, `assets/sorla/model.cbor`, `assets/sorla/agent-gateway.json`, and `assets/sorx/start.schema.json`.
- The fixture suite can be extended with future SorLa features without changing manager architecture.

## Non-goals

- Do not build a demo app around one business domain.
- Do not special-case field names beyond generic type/metadata inference.
- Do not add provider-specific fixture dependencies unless already supported by SORX local/in-memory runtime.
