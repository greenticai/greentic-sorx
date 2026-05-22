# PR: Runtime mapping between deployed views and canonical state

Repo: `greenticai/greentic-sorx`

## Goal
Route requests for `/v1.0`, `/v1.1`, `/v2.0` to one canonical state model using SoRLa-provided mappings.

## Current code assumptions

- Direct, HTTP and MCP invocations currently route to one loaded pack/runtime at a time.
- Deployment records carry `api_version_label` and `base_path`, but route execution is not yet a multi-deployment dispatcher.
- Provider operations are currently pack/version namespaced; PR 07 must make canonical SoR-scoped state available before cross-version visibility can work.
- No canonical-to-view or view-to-canonical mapping runtime exists yet.

## Runtime flow

Read:

```text
resolve route -> deployment -> view version -> load canonical entity -> canonical_to_view -> response
```

Write:

```text
resolve route -> validate view input -> view_to_canonical command -> policy -> transaction -> view response
```

## Acceptance criteria

- Same tenant+sor can expose at least three active view versions.
- v1.0 can be configured read-only.
- v1.1 and v2.0 can write to canonical state.
- Tests prove writes through v1.1 are visible through v2.0 and vice versa where mappings allow.
- Existing direct/HTTP/MCP single-pack runtime behavior remains compatible for local runs.
