# PR: Add graph traversal and index query runtime paths

Repo: `greenticai/greentic-sorx`

## Goal
Support ontology-backed index and graph queries through the canonical store contract, with FoundationDB as the durable implementation once PR 12 lands.

## Current code assumptions

- Startup provider compatibility already validates string capabilities such as `ontology-scoped-evidence-query` and `entity-link`.
- The runtime provider trait currently has only CRUD plus simple filter query.
- Route metadata does not yet express index or traversal requirements.
- FoundationDB is not wired yet, so memory-provider tests should cover contract behavior before durable FoundationDB tests are enabled.

## Required runtime capabilities

- exact index query
- composite index query
- bounded graph traversal using ontology scope
- hydrate IDs from canonical store
- map canonical results to deployment view version

## Acceptance criteria

- Route metadata can require an index or traversal.
- SORX refuses validation/promotion/runtime startup if required provider capability is absent.
- Provider compatibility uses the existing capability-report machinery instead of a separate validation path.
- Landlord/tenant tests cover:
  - tenants by property
  - active tenancy by unit
  - maintenance requests reachable from landlord via property/unit/tenancy graph
