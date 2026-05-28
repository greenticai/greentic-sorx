# PR-06 — Docs, Rollout, and Non-Goals

## Goal

Document Sorx Business Manager clearly so contributors understand the boundary between SorLa, SORX, manager metadata, Adaptive Cards, policy, locale, and messaging providers.

## Docs to add

Suggested files:

```text
docs/sorx-business-manager.md
docs/sorx-manager-policy.md
docs/sorx-manager-i18n.md
docs/sorx-manager-adaptive-cards.md
docs/sorx-manager-fixtures.md
```

## Explain the product split

```text
SorLa Designer
  design-time editing of records, relationships, policies, actions, and packs

SORX
  runtime execution of packaged SorLa systems

Sorx Business Manager
  runtime business management UI generated from SORX metadata

Greentic messaging providers
  channel delivery and Adaptive Card transformation for Slack/Webex/etc
```

Current codebase alignment to document:

- The local HTTP runtime is implemented in `crates/greentic-sorx-cli/src/http_runtime.rs`; reusable manager models/renderers belong in core, while route wiring remains CLI-local for now.
- Existing SORX HTTP surfaces are `/v1/sorx/...` and generated `/v1/agent/...`; manager docs should describe `/v1/sorx/manager/...` routes unless an alias is explicitly added.
- Current caller context headers are `X-Greentic-Tenant-Id`, `X-Greentic-Caller-Id`, and `X-Greentic-Caller-Role`; manager docs should not introduce incompatible tenant/user header names.
- Existing docs already cover business actions, ontology graph/policy, metrics, validation suites, deployments, security, and observability. Manager docs should link to them instead of restating those subsystems.

## Required non-goals

- SORX does not author SorLa.
- SORX does not implement Slack Block Kit/Webex transformation logic.
- Manager is not a domain-specific admin app.
- Manager does not bypass policy checks.
- Manager does not trust card submit payloads.
- Manager does not require SVG click support inside Adaptive Cards.
- Manager docs do not present CLI i18n catalogs as business/manager translation catalogs.

## Rollout plan

### Stage 1

- context model
- policy-filtered manager view
- locale resolution
- canonical Adaptive Card dashboard/create/detail cards
- generic fixtures

### Stage 2

- richer relationship cards
- graph JSON/SVG rendering
- picker endpoints
- action/approval card flows

### Stage 3

- richer dashboards/projections
- audit panels
- advanced localization/formatting
- hosted manager shell improvements

## Acceptance criteria

- Docs describe how to call `/v1/sorx/manager/...` routes.
- Docs show how tenant/team/channel/locale context is resolved.
- Docs preserve existing `X-Greentic-Tenant-Id`, `X-Greentic-Caller-Id`, and `X-Greentic-Caller-Role` conventions.
- Docs explicitly state messaging providers own channel transformations.
- Docs explain generic fixture strategy.
- Docs warn that render-time filtering is not security enforcement.
