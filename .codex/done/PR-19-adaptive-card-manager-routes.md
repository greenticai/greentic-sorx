# PR-04 — Add Manager Routes and Canonical Adaptive Card Rendering

## Goal

Expose Sorx Business Manager through the existing SORX HTTP runtime using canonical Adaptive Cards and manager metadata.

Greentic messaging providers already handle Slack/Webex/etc transformations, so this PR should not implement channel transformation logic.

## Routes

Add routes to the existing local HTTP runtime in `crates/greentic-sorx-cli/src/http_runtime.rs`.

The current SORX-owned routes use `/v1/sorx/...` and generated agent routes use `/v1/agent/...`. Keep the manager surface under `/v1/sorx/manager/...` for consistency, with optional `/manager` convenience alias only if it does not conflict with existing generated routes.

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

## Route behavior

Each route must:

1. Resolve `SorxManagerContext`.
2. Generate base manager view from runtime metadata.
3. Apply policy filtering.
4. Localize labels/messages.
5. Render canonical Adaptive Card JSON or manager JSON.

## Manager shell

For first version, serve a minimal HTML shell at `/v1/sorx/manager` that can show:

- dashboard card JSON
- relationship SVG/image
- links to manager metadata

This is only a convenience shell. The primary output is channel-portable card payloads.

## Card renderer

Suggested location:

```text
crates/greentic-sorx-core/src/manager/cards.rs
```

Keep card/view rendering pure and provider-neutral in core. Route wiring, HTTP auth, header parsing, and JSON/HTML responses belong in the CLI HTTP adapter unless/until the repo grows a core HTTP server abstraction.

Renderers:

- dashboard card
- record list card
- create record card
- detail card
- relationship summary card
- approval/action card placeholder

## Submit handling

`POST /v1/sorx/manager/submit` must never trust the card payload.

It must:

1. Resolve actor/context again.
2. Re-evaluate policy.
3. Route to existing `SorxRuntime::invoke` operation path.
4. Preserve provider resolution, idempotency, policy checks, and audit behavior.
5. Return a next card or structured result.

## Relationship graph

Add:

```text
GET /v1/sorx/manager/graph.json
GET /v1/sorx/manager/graph.svg
```

`graph.json` should reuse existing ontology graph assets/services where present (`assets/sorla/ontology.graph.json`, `OntologyGraphService`) and derive simple reference edges from endpoint/entity metadata only when ontology metadata is absent. `graph.svg` is a convenience rendering.

Do not depend on clickable SVG inside Adaptive Cards. Relationship cards should provide buttons/drill-down actions instead.

## Acceptance criteria

- Routes compile and can be exercised in tests.
- Cards are generated from manager metadata, not hardcoded domain templates.
- Submit path re-checks policy.
- Adaptive Card JSON is canonical and provider-neutral.
- Existing `/v1/sorx/...` and `/v1/agent/...` behavior is unchanged.
- No Slack/Webex transformation code added.
