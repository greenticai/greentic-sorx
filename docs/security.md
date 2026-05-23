# Security Model

`greentic-sorx` treats `.gtpack` files and startup answers as untrusted input.
The current runtime is local-first, but the checks are shaped for later
deployment registry and public rollout work.

## Pack Loading

- Only `.gtpack` archives are accepted by the pack loader.
- Archive entries must be relative paths. Absolute paths, backslashes, and
  `..` components are rejected before assets are read.
- Manifest asset references must stay under `assets/sorla/`, `assets/sorx/`,
  or known manifest/lock paths.
- `pack.lock.cbor` entries are verified for size and SHA-256 digest when the
  lock file is present.
- Manifest `integrity.digest`, `integrity.signature`, and
  `integrity.signature_ref` are parsed for future signing validation. They are
  not yet enforcement gates.

## Startup Answers

- Schema defaults are applied deterministically.
- Unknown answer keys are rejected unless the embedded schema permits additional
  properties.
- Secret-like keys such as `password`, `api_key`, `client_secret`, and token
  fields must use references such as `secret:`, `env:`, `vault:`, `ref:`, or
  `${...}`.
- Direct provider `config` is accepted only in `local` or `test`
  environments. Other environments must use `config_ref`.
- Normalized answers and dry-run plans expose whether direct config exists, but
  should not be used to print secret material.

## Runtime Requests

- Outside `local`, HTTP requests must include `X-Greentic-Tenant-Id` and
  `X-Greentic-Caller-Id`.
- HTTP ingest can be protected with startup answers
  `server.auth.mode = "shared_secret"` and
  `server.auth.shared_secret_ref = "env:SORX_HTTP_INGEST_SECRET"`. When
  enabled, SORX APIs and generated routes require either
  `Authorization: Bearer <secret>` or `X-Greentic-Sorx-Secret: <secret>`;
  health probes remain unauthenticated.
- Generated routes use the endpoint input schema from `agent-gateway.json` for
  required-field and scalar validation.
- Locked business action invocation uses explicit URL action id/version plus
  the request contract hash. Labels, aliases, and descriptions are discovery
  metadata only and are not used for runtime action selection.
- Locked business action contract drift fails closed before provider invocation.
  Missing, malformed, and mismatched contract hashes have distinct stable error
  codes.
- Business action dry-runs validate the payload, policy, provider binding, and
  execution target without mutating provider state or writing idempotency state.
- Mutating operations should use `Idempotency-Key`. SORX scopes create
  idempotency by operation so one key cannot cross operation boundaries.
- Request bodies are not included in audit events by default.
- Ontology graph and evidence commands enforce static ontology policy hints
  before relationship traversal or evidence retrieval.
- Ontology policy decisions can return redaction metadata for sensitive fields
  and deny evidence or traversal over restricted concepts.
- Ontology/evidence explain and audit payloads carry hashes, IDs, and counts;
  request bodies and secret values are not added to those command audit events.
- Public promotion for ontology-enabled validation reports requires ontology
  static validation, provider compatibility, retrieval binding validation, and
  ontology policy validation gates to pass unless a local operator override is
  recorded.

## Policy Defaults

- Low and medium risk operations execute by default.
- High risk operations require approval by default.
- Critical operations are denied by default.
- Endpoint metadata can require approval even when a risk level is configured
  for automatic execution.

## Current Gaps

- Full signature verification is deferred to the deployment/versioning work.
- External approval systems are not wired yet; local brokers cover auto,
  pending, and deny behavior.
- FoundationDB execution is blocked until a SORX-compatible store provider is
  exposed by the provider repository.
