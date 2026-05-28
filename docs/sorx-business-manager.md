# Sorx Business Manager

Sorx Business Manager is a runtime management surface generated from packaged SorLa/SORX metadata. It does not author SorLa packs; SorLa Designer remains the design-time tool for records, relationships, policies, actions, and package content.

SORX owns runtime execution of packaged systems, policy/approval/audit enforcement, provider routing, and manager metadata/card generation. Greentic messaging providers own channel delivery and any Slack, Webex, or other channel-specific transformations.

## Routes

The local HTTP runtime exposes manager routes under the existing SORX namespace:

```text
GET  /v1/sorx/manager
GET  /v1/sorx/manager/view
GET  /v1/sorx/manager/cards/dashboard
GET  /v1/sorx/manager/cards/records/{record}
GET  /v1/sorx/manager/cards/records/{record}/create
GET  /v1/sorx/manager/cards/records/{record}/{id}
GET  /v1/sorx/manager/cards/relationships
GET  /v1/sorx/manager/graph.json
GET  /v1/sorx/manager/graph.svg
GET  /v1/sorx/manager/pickers/{record}
POST /v1/sorx/manager/submit
```

Existing `/v1/sorx/...` diagnostics and generated `/v1/agent/...` routes are unchanged.

## Context Headers

Manager requests preserve existing SORX HTTP context conventions:

```text
X-Greentic-Tenant-Id: tenant-a
X-Greentic-Caller-Id: user-or-service
X-Greentic-Caller-Role: reader,approver
X-Greentic-Team: team-alpha
X-Greentic-Channel: web|api|teams|slack|webex|webchat
X-Greentic-Sor: optional-sor-id
Accept-Language: fr-FR,fr;q=0.8
```

Outside local mode, tenant and caller headers are required. In local mode the runtime can use startup-answer defaults.

## Submit Safety

`POST /v1/sorx/manager/submit` does not trust card data as an authorization grant. It resolves context again and delegates to `SorxRuntime::invoke`, preserving provider binding resolution, idempotency, policy decisions, approvals, and audit behavior.

Related docs:

- [Business actions](business-actions.md)
- [Ontology policy](ontology-policy.md)
- [Provider bindings](provider-bindings.md)
- [Security](security.md)
