# greentic-sorx

`greentic-sorx` is the Greentic System of Record eXecutor. In plain terms:
it takes a packaged SorLa system and runs it as a real, validated runtime.

SorLa describes a system of record: the business objects, operations, routes,
MCP tools, validation rules, and runtime metadata for something like a tenant
registry, invoice approval system, customer profile store, or any other
structured operational system.

SORX does not author SorLa. SORX runs SorLa after it has been packaged into a
`.gtpack` file.

## The Short Version

The usual flow is:

1. Write or generate a SorLa system.
2. Package it with `greentic-sorla` into a `.gtpack`.
3. Check the pack with `greentic-sorx doctor`.
4. Ask SORX what startup answers it needs.
5. Start SORX with those answers.
6. SORX exposes the pack as HTTP routes and MCP tool metadata backed by a
   provider such as memory or, later, FoundationDB.

```bash
greentic-sorla pack landlord.sorla --out landlord.gtpack
greentic-sorx doctor landlord.gtpack
greentic-sorx start landlord.gtpack --schema > landlord.sorx.schema.json
greentic-sorx start landlord.gtpack --answers landlord.sorx.answers.json
```

## What Is SorLa?

SorLa is Greentic's language and packaging model for declaring a system of
record.

A SorLa pack can say:

- what entities exist, such as tenants, leases, invoices, or approvals
- what operations are allowed, such as create, update, query, or approve
- what HTTP routes and MCP tools should be exposed
- what startup configuration is required
- what validations must pass before a pack can be trusted
- how entities map to storage providers and collections

SorLa itself is the design and build side. SORX is the execution side.

## What Is a `.gtpack`?

A `.gtpack` is the handoff artifact between SorLa and SORX. It is a zip-based
Greentic pack that contains the runtime files SORX needs, including SorLa
assets such as:

- `assets/sorla/agent-gateway.json` for generated endpoint routes
- `assets/sorla/mcp-tools.json` for MCP tool metadata
- startup answer schemas
- validation suites
- pack metadata and lock files

SORX deliberately uses `.gtpack` files as its runtime input. It does not treat a
loose build folder as the main runtime contract.

## What SORX Does With a Pack

When SORX starts a pack, it:

- validates the archive and runtime metadata
- reads the startup schema and checks the supplied answers
- applies safe defaults from the schema
- rejects inline secret-like answer values
- resolves provider bindings for SorLa entities
- builds endpoint routes from the pack metadata
- loads MCP tool metadata
- enforces risk policy and approval rules
- emits structured audit events
- starts a local HTTP runtime for the pack

The in-memory provider is implemented and used for local development and CI.
The FoundationDB adapter currently accepts the config boundary but returns a
clear unavailable error until a SORX-compatible store provider exists in the
provider stack.

## Install

After a release, users can install the CLI with either:

```bash
cargo install greentic-sorx
cargo binstall greentic-sorx
```

For local development in this repository:

```bash
cargo build
cargo run --bin greentic-sorx -- --help
```

Localized help is built into the binary:

```bash
cargo run --bin greentic-sorx -- --help --locale nl
cargo run --bin greentic-sorx -- --help --locale es
```

## Common Commands

Inspect and validate a pack:

```bash
greentic-sorx doctor landlord.gtpack
greentic-sorx doctor landlord.gtpack --json
greentic-sorx inspect landlord.gtpack
greentic-sorx routes landlord.gtpack --json
greentic-sorx mcp-tools landlord.gtpack
```

Prepare and start a runtime:

```bash
greentic-sorx start landlord.gtpack --schema --json
greentic-sorx start landlord.gtpack --answers landlord.sorx.answers.json
greentic-sorx start landlord.gtpack --answers landlord.sorx.answers.json --dry-run --json
greentic-sorx start landlord.gtpack --answers landlord.sorx.answers.json --emit-answers
greentic-sorx run landlord.gtpack --answers landlord.sorx.answers.json
```

Run pack validation suites:

```bash
greentic-sorx validate landlord.gtpack --answers landlord.sorx.answers.json --provider-mode in-memory --json
```

Work with local deployments and aliases:

```bash
greentic-sorx deployments list
greentic-sorx deployments create \
  --pack landlord.gtpack \
  --tenant acme \
  --sor landlord \
  --environment production \
  --api-version v1 \
  --base-path /sorx/acme/landlord/v1 \
  --visibility private
greentic-sorx deployments promote <deployment-id> --alias latest --public
greentic-sorx deployments rollback --tenant acme --sor landlord --alias latest --to <previous-deployment-id>
greentic-sorx aliases list --tenant acme
```

Exercise the GitHub/GHCR webhook fixture path:

```bash
greentic-sorx webhook verify-fixture fixtures/github-ghcr-published.json
greentic-sorx webhook replay fixtures/github-ghcr-published.json
```

## What the Local HTTP Runtime Serves

When `start --answers` runs, the local HTTP runtime serves:

- `GET /healthz`
- `GET /readyz`
- `GET /v1/sorx/routes`
- `GET /v1/sorx/public-routes`
- `GET /v1/sorx/tools`
- `GET /v1/sorx/deployments/local/routes`
- `GET /v1/sorx/deployments/local/promotion-status`
- generated routes declared by the pack's `agent-gateway.json`

Mutating routes go through the same runtime path as MCP adapter calls: provider
resolution, policy checks, idempotency handling, and audit events.

## Business Events

SORX publishes business events as canonical Greentic `EventEnvelope`s
(from `greentic-types`) on every record create, update, and delete, and
whenever a command step runs `emit_event`.  Publication is best-effort and
never fails the originating business operation.

### Configuration

Add an `events` section to your startup answers file:

```yaml
events:
  sink: nats            # disabled (default) | stdout | nats
  nats_url: "nats://localhost:4222"
  subject_prefix: "greentic.events"
```

The `sink` field selects the delivery backend.  `disabled` (the default)
discards all events silently.  `stdout` prints JSON envelopes to standard
output and is useful for local development.  `nats` requires the
`events-nats` cargo feature (not included in the default build) and a valid
`nats_url`.

### Topics and NATS subjects

Each event carries a **topic** that follows one of two patterns:

- Entity lifecycle events: `sorla.<pack>.<Entity>.<operation>` — for example
  `sorla.landlord.Tenant.created`.  The operation is one of `created`,
  `updated`, or `deleted`.
- Command-emitted (domain) events: `sorla.<pack>.<event_name>` — for example
  `sorla.landlord.RecordRemoved`.

Topic segments are sanitized: any character that is not ASCII alphanumeric,
a hyphen, or an underscore is replaced with a hyphen.

When the NATS sink is active, each topic is published to a NATS subject of
the form `<subject_prefix>.<tenant>.<topic>` — for example
`greentic.events.acme.sorla.landlord.Tenant.created`.

### Delivery contract

Delivery is at-most-once.  A full queue or NATS outage causes events to be
dropped with a log line; the business operation is never rolled back or
retried.  Command events remain persisted in the canonical store regardless
of sink availability.

One important nuance for the NATS sink: if the initial NATS connection at
startup fails, the background publisher exits and events are dropped with log
lines until the process is restarted.  After a successful initial connection,
`async-nats` handles transient outages and reconnects automatically.

### Discovery

Topics are advertised as capability offers in the `/v1/sorx/routes` response
under the contract `greentic.sorx.business-event-topic.v1`.  Each offer
includes a `topic` metadata field containing the exact topic string so
consumers can subscribe without guessing the sanitized form.

### Feature flag

The NATS sink is compiled in only when the `events-nats` cargo feature is
enabled:

```bash
cargo build --features events-nats
```

The `disabled` and `stdout` sinks are always available regardless of features.

## Startup Answers

SORX starts from an answer file. The file can be either the raw SORX answer
object or a `greentic-qa` style `AnswerSet` envelope with `form_id`,
`spec_version`, and `answers`.

Provider answers may include `bindings.entities` to map SorLa entities to
provider IDs and collections. If bindings are omitted, SORX derives safe
defaults from the pack's gateway metadata.

Use `config_ref` for normal provider configuration. Direct provider `config`
values are accepted only for local or test startup answers.

## Exit Codes

SORX uses stable exit codes so scripts and agents can react predictably:

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Generic error |
| 2 | Invalid CLI usage |
| 3 | Pack validation failed |
| 4 | Startup answers validation failed |
| 5 | Provider resolution failed |
| 6 | Runtime startup failed |
| 7 | Policy denied during dry-run |

## Repository Layout

- `crates/greentic-sorx-core`: runtime types, startup planning, providers,
  policy, approvals, audit, endpoint routing, deployment registry, webhook
  handling, and in-memory execution
- `crates/greentic-sorx-pack`: `.gtpack` loading, inspection, doctor checks,
  and runtime metadata validation
- `crates/greentic-sorx-cli`: the `greentic-sorx` binary, CLI commands,
  localized help, local HTTP adapter, validation command, and MCP adapter plan
  surface
- `i18n/`: source CLI translations
- `crates/greentic-sorx-cli/i18n/`: synced translation files embedded in the
  published CLI crate and binary
- `docs/`: detailed behavior notes for commands, answers, deployments, MCP,
  validation, security, observability, and release readiness
- `ci/`: local and CI validation scripts

## Notes for Coding Agents

If you are changing this repo, keep these boundaries in mind:

- SORX consumes `.gtpack` files. Do not add loose-folder runtime behavior unless
  the task explicitly asks for it.
- `greentic-sorla` owns authoring, parsing, canonical IR, and pack production.
  `greentic-sorx` owns runtime loading, validation, startup answers, HTTP/MCP
  execution, providers, policy, approval, audit, deployments, and local/e2e
  execution.
- The in-memory provider is real and should stay deterministic.
- The FoundationDB adapter is intentionally an unavailable boundary until a
  SORX-compatible CRUD store contract exists in the provider repos.
- `mcp start` currently validates answers and emits an adapter runtime plan.
  Full MCP server transport is future work.
- `validate` runs declarative validation suites embedded in packs. It must not
  run arbitrary pack code.
- Public deployment promotion is gated by a passing validation report for the
  same deployment ID and pack digest.
- Keep CLI translations in sync with:

  ```bash
  tools/i18n.sh validate
  tools/i18n.sh status
  bash tools/sync_cli_i18n.sh
  ```

- Run the release metadata guard after touching Cargo metadata or workflows:

  ```bash
  bash ci/release_check.sh
  ```

## Development Checks

Run the same checks locally that CI runs:

```bash
bash ci/local_check.sh
```

This runs formatting, i18n validation, release metadata validation, clippy,
tests, build, docs, and crates.io packaging checks for publishable crates.

The landlord/tenant e2e scenario can be run with:

```bash
bash scripts/e2e/run-landlord-tenant.sh --provider memory
```

## Releases

To cut a release:

1. Bump the package version in `Cargo.toml`.
2. Commit the version change.
3. Push the commit to `main`:

   ```bash
   git push origin main
   ```

4. `.github/workflows/publish.yml` verifies the version, runs the full local
   check, creates or verifies the matching `vX.Y.Z` tag, dispatches
   `.github/workflows/release-binaries.yml` on that tag, waits for the six
   GitHub Release archives to upload for `cargo binstall`, then dry-runs
   package publishing and publishes crates to crates.io in dependency order.
5. `.github/workflows/release-binaries.yml` can also be manually dispatched to
   rebuild GitHub Release archives for an existing tag.

Required GitHub secret:

- `CARGO_REGISTRY_TOKEN` for crates.io publishing

GHCR publishing is intentionally disabled for this repo. Future SORX runtime
work may read GHCR OCI references and exact digests when implementing
deployment webhooks and registries.

## More Documentation

Start with:

- `docs/getting-started.md`
- `docs/commands.md`
- `docs/answers.md`
- `docs/provider-bindings.md`
- `docs/mcp.md`
- `docs/deployments.md`
- `docs/validation-suites.md`
- `docs/security.md`
- `docs/observability.md`
- `docs/release.md`

Audit and design notes:

- `docs/audit/reuse-audit.md`
- `docs/audit/provider-foundationdb-gap-pr08.md`
- `docs/audit/gtc-integration-pr10.md`
- `docs/design/sorx-gtbundle-integration.md`
