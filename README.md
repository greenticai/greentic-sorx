# greentic-sorx

`greentic-sorx` is the Greentic System of Record eXecutor. It is intended to
consume SoRLa `.gtpack` artifacts and run them as local or deployed systems of
record with runtime validation, startup answers, HTTP/MCP surfaces, provider
bindings, policy enforcement, approvals, and audit.

This repository currently contains the PR 01 scaffold through the PR 11 CI,
documentation, security, determinism, and release hardening pass: a small Rust
workspace with core, CLI, and `.gtpack` runtime pack support crates.
The `doctor`, `inspect`, startup schema/answer commands, internal
provider-backed runtime invocation path, generated route listing, local HTTP
endpoint server, risk policy decisions, local approval brokers, idempotent
creates, structured audit events, MCP tool metadata loading, MCP runtime
adapter, entity binding resolution, tenant/pack/version provider namespaces, and
memory-backed provider execution perform real validation/planning/execution.
The FoundationDB adapter currently reports a clear unavailable error until a
SORX-compatible store provider is exposed; full MCP server transport and
deployment lifecycle behavior remain planned for later PRs.

## Runtime Boundary

SORX consumes `.gtpack` files as the runtime handoff contract. It does not use a
loose `./dist` folder as its primary runtime input.

`greentic-sorla` owns authoring, parsing, canonical IR, and `.gtpack`
production. `greentic-sorx` owns runtime loading, validation, startup answers,
HTTP/MCP execution, provider binding, policy, approval, audit, and local/e2e
execution.

## Workspace

- `crates/greentic-sorx-core`: shared SORX version/context/command types, startup answer normalization/planning, endpoint routing, provider traits/binding resolution, policy/approval/audit abstractions, MCP adapter support, and in-memory runtime execution.
- `crates/greentic-sorx-pack`: SoRLa `.gtpack` loader, inspector, doctor, and runtime metadata validation.
- `crates/greentic-sorx-cli`: the `greentic-sorx` binary, CLI parser, route/tool listing, local HTTP runtime adapter, and MCP adapter plan surface.

## Command Surface

Minimal flow:

```bash
greentic-sorla pack examples/landlord.sorla --out landlord.gtpack
greentic-sorx doctor landlord.gtpack
greentic-sorx start landlord.gtpack --schema > sorx.schema.json
greentic-sorx start landlord.gtpack --answers examples/landlord.answers.json
```

```bash
greentic-sorx --help
greentic-sorx --version
greentic-sorx doctor landlord.gtpack
greentic-sorx doctor landlord.gtpack --json
greentic-sorx inspect landlord.gtpack
greentic-sorx routes landlord.gtpack --json
greentic-sorx mcp-tools landlord.gtpack
greentic-sorx deployments list
greentic-sorx deployments create --pack landlord.gtpack --tenant acme --sor landlord --environment production --api-version v1 --base-path /sorx/acme/landlord/v1 --visibility private
greentic-sorx deployments promote <deployment-id> --alias latest --public
greentic-sorx deployments rollback --tenant acme --sor landlord --alias latest --to <previous-deployment-id>
greentic-sorx aliases list --tenant acme
greentic-sorx webhook verify-fixture fixtures/github-ghcr-published.json
greentic-sorx validate landlord.gtpack --answers landlord.answers.json --provider-mode in-memory --json
greentic-sorx start landlord.gtpack --schema --json
greentic-sorx start landlord.gtpack --answers landlord.sorx.answers.json
greentic-sorx start landlord.gtpack --answers landlord.sorx.answers.json --dry-run --json
greentic-sorx start landlord.gtpack --answers landlord.sorx.answers.json --emit-answers
greentic-sorx run landlord.gtpack --answers landlord.sorx.answers.json
greentic-sorx mcp start landlord.gtpack --answers landlord.sorx.answers.json
```

`doctor` validates a `.gtpack` archive and `inspect` emits stable JSON metadata.
`routes <pack.gtpack>` emits generated route metadata from the pack
`agent-gateway.json`. `mcp-tools <pack.gtpack>` emits the resolved
`greentic.sorx.mcp-tools.v1` tool list from `assets/sorla/mcp-tools.json`.
`start --schema` emits the pack startup schema. `start --answers` validates
answer files, applies schema defaults, rejects inline secret-like values, and
starts a local HTTP server. Use `--dry-run` to emit a deterministic startup
plan, or `--emit-answers` to emit normalized full answers. `run --answers` is an
alias for the same HTTP startup path. `mcp start --answers` validates answers
and emits an adapter-only MCP runtime plan with resolved tools; a full MCP
server transport is a follow-up.

`deployments` and `aliases` manage the local deployment registry. Use
`--registry <path>` or `SORX_REGISTRY_PATH` to select the registry file. Public
promotion is gated by a passing validation report for the same deployment ID
and pack digest, and rollback moves aliases without deleting deployment
records.

`webhook verify-fixture` and `webhook replay` exercise the PR13 GitHub/GHCR
publish callback path with a fake OCI resolver. Real GHCR auth/download wiring
is intentionally deferred behind the resolver trait.

`validate` executes a pack-embedded declarative validation suite and emits a
stable validation report. It never runs arbitrary pack code.

Stable exit codes are `0` success, `1` generic error, `2` invalid CLI usage,
`3` pack validation failed, `4` answers validation failed, `5` provider
resolution failed, `6` runtime startup failed, and `7` policy denied during
dry-run.

The local HTTP runtime serves:

- `GET /healthz`
- `GET /readyz`
- `GET /v1/sorx/routes`
- `GET /v1/sorx/public-routes`
- `GET /v1/sorx/tools`, returning resolved MCP tool metadata
- `GET /v1/sorx/deployments/local/routes`
- `GET /v1/sorx/deployments/local/promotion-status`
- generated agent endpoint routes from `agent-gateway.json`

Startup answer files may be either the raw SORX answer object or a
`greentic-qa`-style `AnswerSet` envelope containing `form_id`, `spec_version`,
and `answers`.

Current `gtc` supports fixed passthroughs and extension handoff files, but not a
generic `gtc sorx` route yet. Future `gtc` extension usage should follow the
same shape once `gtc sorx` routing or generic `greentic-*` discovery is wired:

```bash
greentic-sorla pack landlord.sorla --out landlord.gtpack
gtc sorx start landlord.gtpack --schema
gtc sorx start landlord.gtpack --answers landlord.sorx.answers.json
```

Provider startup answers may include `bindings.entities` to map SoRLa entities
to provider IDs and collections. If bindings are omitted, SORX derives safe
defaults from the gateway entity/collection metadata. Provider `config_ref` is
preferred for normal runtime use; direct provider `config` is accepted only for
local/test startup answers.

The landlord/tenant e2e scenario can be run with:

```bash
bash scripts/e2e/run-landlord-tenant.sh --provider memory
```

See `docs/getting-started.md`, `docs/commands.md`, `docs/answers.md`,
`docs/provider-bindings.md`, `docs/mcp.md`, `docs/deployments.md`,
`docs/ghcr-webhooks.md`, `docs/validation-suites.md`, `docs/security.md`,
`docs/observability.md`, `docs/future-signing-and-versioning.md`, and
`docs/release.md` for the PR 11 user, security, observability, and release
hardening docs.
See `docs/audit/reuse-audit.md` for the initial Greentic reuse audit and
`docs/audit/provider-foundationdb-gap-pr08.md` for the PR 08 FoundationDB
adapter gap. See `docs/e2e-landlord-tenant.md` for the PR 09 e2e scenario.
See `docs/audit/gtc-integration-pr10.md` and
`docs/design/sorx-gtbundle-integration.md` for PR 10 `gtc`/bundle integration
guidance.

## CI and Releases

Run the same checks locally that CI runs:

```bash
bash ci/local_check.sh
```

The local check runs formatting, clippy, tests, build, docs, and crates.io packaging dry-run checks for publishable crates.

Pull requests and pushes to `main`/`master` run `.github/workflows/ci.yml`.

Nightly coverage enforcement runs through `.github/workflows/nightly-coverage.yml` and checks `coverage-policy.json` with `greentic-dev coverage`.

CLI i18n strings start in `i18n/en.json`. Use the Greentic translator workflow to update every supported locale in 200-key batches:

```bash
tools/i18n.sh translate
tools/i18n.sh validate
tools/i18n.sh status
```

To cut a release:

1. Bump the package version in `Cargo.toml`.
2. Commit the version change.
3. Create and push a matching tag:

   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

4. `.github/workflows/publish.yml` verifies that the tag matches the Cargo version, runs the full local check, builds and uploads the six GitHub Release archives for `cargo binstall`, then dry-runs package publishing and publishes to crates.io in dependency order.
5. `.github/workflows/release-binaries.yml` is a manual helper for rebuilding GitHub Release archives.

After release, users can install with either:

```bash
cargo install greentic-sorx
cargo binstall greentic-sorx
```

Required GitHub secret:

- `CARGO_REGISTRY_TOKEN` for crates.io publishing.

GHCR publishing is intentionally disabled for this repo. Future SORX runtime work may read GHCR OCI references and exact digests when implementing deployment webhooks and registries.
