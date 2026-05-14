# Repository Overview

## 1. High-Level Purpose

`greentic-sorx` is intended to become the Greentic System of Record eXecutor: a Rust executable that consumes SoRLa `.gtpack` artifacts and runs them through runtime validation, startup answers, HTTP/MCP endpoints, provider bindings, policy/approval enforcement, audit logging, and deployment lifecycle management.

The current repository has completed the PR 01 scaffold, PR 02 pack loader/doctor work, PR 03 startup schema/answer planning work, PR 04 runtime core/provider work, PR 05 HTTP endpoint runtime work, PR 06 policy/approval/audit work, PR 07 MCP tool adapter work, PR 08 provider binding/FoundationDB adapter boundary work, PR 09 landlord/tenant e2e work, PR 10 `gtc`/bundle integration alignment work, PR 11 CI/docs/security/release hardening work, PR 12 concurrent deployment registry work, PR 13 GHCR publish webhook-to-pending-deployment work, PR 14 pack-embedded validation-suite work, and PR 15 public promotion/rollback gate work. It is now a small Rust workspace with core types, startup answer normalization/planning helpers, endpoint routing, provider traits, entity binding resolution, tenant/pack/version provider namespaces, policy enforcement, local approval brokers, audit sinks, an in-memory provider, a FoundationDB unavailable adapter boundary, generated route listing, a local HTTP runtime adapter, MCP tool metadata loading, an MCP runtime adapter, a landlord/tenant e2e scenario, stable CLI exit-code behavior, a local JSON deployment registry with validation-gated public promotion, aliases, rollback audit, and deployment-scoped route listing, signed GitHub/GHCR webhook fixture handling with replay protection, declarative pack-embedded validation suite execution, `gtc`/`.gtbundle` integration docs, security/user/release docs, a CLI, a `.gtpack` loader/inspector/doctor crate, local/CI/e2e/release automation, lightweight performance guard tests, i18n assets, and `.codex` PR specifications that describe the remaining SORX implementation path.

## 2. Main Components and Functionality

- **Path:** `Cargo.toml`
  - **Role:** Defines the Rust workspace.
  - **Key functionality:**
    - Workspace members are `crates/greentic-sorx-core`, `crates/greentic-sorx-pack`, and `crates/greentic-sorx-cli`.
    - Shared package metadata is defined for version, edition, license, repository, docs, keywords, and categories.
    - Shared dependencies include `clap`, `criterion`, `serde`, `ciborium`, `zip`, `sha2`, and local workspace crates.

- **Path:** `crates/greentic-sorx-core`
  - **Role:** Core SORX runtime types and adapters.
  - **Key functionality:**
    - Defines `SorxVersion`, `SorxCommandContext`, and `SorxCommand`.
    - Defines startup answer normalization, default application, basic schema validation, secret-like answer rejection, runtime config construction, and deterministic dry-run startup plan generation.
    - Defines `SorxRuntimeConfig` and related server, MCP, provider, policy, audit, deployment, exposure, GHCR, and GHCR webhook configuration structs.
    - Defines runtime models for endpoint definitions, invocations, results, caller context, events, risk levels, methods, and operation kinds.
    - Builds `EndpointRouter` instances from SoRLa `agent-gateway.json` metadata.
    - Defines the local `SorStoreProvider` trait, CRUD/query operation structs, `ProviderRegistry`, deterministic `MemoryStoreProvider`, provider kind parsing, `ProviderNamespace`, `ProviderBinding`, and `BindingResolver`.
    - Parses startup `bindings.entities` config for entity-to-provider/collection resolution, derives defaults from gateway metadata when bindings are omitted, and fails clearly when explicit bindings omit an invoked entity.
    - Applies tenant/pack/version namespaces to provider operations so local providers can isolate records per tenant and pack version.
    - Provides a `FoundationDbProviderAdapter` boundary that accepts `config_ref` or local/test direct config and returns `provider_unavailable` until a SORX-compatible FoundationDB store provider is wired.
    - Defines risk policy types, `PolicyEngine`, policy decisions, approval broker trait, local auto-approve/deny/pending brokers, structured `SorxAuditEvent`, and stdout/memory/disabled audit sinks.
    - Loads and validates MCP tool definitions from SoRLa `mcp-tools.json` metadata.
    - Provides an `McpRuntime` adapter that invokes MCP tools through the same `SorxRuntime` router, policy, provider, approval, idempotency, and audit path as direct and HTTP calls.
    - Defines PR 12/15 deployment registry types for immutable pack artifacts, deployments, aliases, public route tables, deployment statuses, visibility, state modes, traffic split metadata, validation-report gates, promotion audit events, rollback requests, conflict checks, and local JSON registry storage.
    - Defines PR 13/15 GitHub/GHCR webhook types for config, headers, payload metadata, promotion policy, OCI references, resolved OCI artifacts, resolver trait, signed webhook handling, and fixture-friendly outcomes.
    - Executes endpoint invocations through `SorxRuntime` without HTTP/MCP by validating input, deciding policy before provider execution, requesting approvals when needed, resolving provider bindings, mapping operations, emitting structured audit events, and returning endpoint results.
    - Records invocation source as direct, HTTP, or MCP in runtime/audit models.
    - Scopes create idempotency keys by operation before passing them to providers.
    - Supports strict router validation for missing risk metadata on mutating gateway operations.
    - Keeps `LoadedSorlaPack` as a placeholder type; real pack loading lives in `greentic-sorx-pack`.
    - Includes unit/integration tests for version/context behavior, startup answer validation/planning, router construction, provider-backed create/get/update/query, missing providers, unknown endpoints, invalid input, deterministic ordering, idempotent create behavior, MCP metadata loading, MCP provider invocation, approval-required MCP calls, audit source recording, MCP/direct result equivalence, provider binding resolution, namespace propagation, memory provider namespace isolation, FoundationDB unavailable behavior, provider config mode validation, concurrent deployment creation, deployment conflict checks, alias resolution, retirement, local registry persistence, webhook HMAC validation, trust checks, exact digest checks, replay rejection, and pending deployment creation from GHCR publish fixtures.
  - **Key dependencies / integration points:** Intended as the shared base for loader/runtime/provider/policy/MCP adapter work.

- **Path:** `crates/greentic-sorx-pack`
  - **Role:** SoRLa `.gtpack` loader, inspector, and doctor support for SORX runtime packs.
  - **Key functionality:**
    - Opens only `.gtpack` archive files and rejects missing paths, non-`.gtpack` paths, invalid archives, absolute paths, backslashes, and `..` traversal entries.
    - Reads `pack.cbor` into a minimal `PackManifest` and optional `pack.lock.cbor` into a minimal `PackLock`.
    - Parses future manifest integrity metadata fields for digest/signature references without enforcing them yet.
    - Validates the SORX runtime extension marker `greentic.sorx.runtime.v1`.
    - Enforces required runtime entries: `pack.cbor`, `assets/sorla/model.cbor`, `assets/sorla/agent-gateway.json`, and `assets/sorx/start.schema.json`.
    - Validates manifest/extension asset references stay under allowed pack paths and point to existing entries.
    - Checks lock entry sizes and SHA-256 digests when `pack.lock.cbor` is present.
    - Reads required SoRLa and SORX assets; parses JSON/YAML assets where appropriate.
    - Validates optional `assets/sorla/mcp-tools.json` metadata for schema shape, duplicate tool names, input schema object shape, and endpoint/operation references against `agent-gateway.json`.
    - Preserves optional validation-suite assets, JSON fixture contents, and reports validation-suite status as `missing`, `present`, or `invalid`.
    - Emits warning-only doctor findings for missing lock files and obvious secret-like markers in runtime/provider templates.
    - Provides stable inspect JSON through `SorxInspectReport`.
    - Includes unit tests that build synthetic `.gtpack` fixtures for valid packs, missing assets, invalid JSON, path traversal, optional MCP tools, invalid MCP references, duplicate MCP tool names, validation-suite status/schema checks, fixture loading, and future integrity metadata parsing.
  - **Key dependencies / integration points:** Uses `zip`, `ciborium`, `serde_json`, `serde_yaml`, and `sha2`. It deliberately implements only the minimal local reader needed by SORX until a stable shared Greentic pack-reader API is available.

- **Path:** `crates/greentic-sorx-cli`
  - **Role:** CLI package and `greentic-sorx` binary.
  - **Key functionality:**
    - Uses `clap` to parse `doctor`, `inspect`, `routes`, `mcp-tools`, `mcp start`, `start`, and `run`.
    - Supports `greentic-sorx --help` and `greentic-sorx --version`.
    - Defines stable process exit codes for success, generic failure, CLI usage errors, pack validation failures, answer validation failures, provider resolution failures, runtime startup failures, and future dry-run policy denial.
    - Wires `doctor <pack.gtpack>` to real pack validation.
    - Wires `inspect <pack.gtpack>` to stable JSON metadata output.
    - Wires `start <pack.gtpack> --schema` to emit the embedded startup schema.
    - Wires `start <pack.gtpack> --answers <file>` to validate and normalize answers, build the runtime config/provider registry, and start a local HTTP server.
    - Accepts provider `config_ref` for normal runtime use and direct provider `config` only in local/test startup answers.
    - Supports `--dry-run` for deterministic startup plan output and `--emit-answers` for normalized answer output.
    - Accepts raw SORX answer objects and `greentic-qa`-style `AnswerSet` JSON envelopes with `form_id`, `spec_version`, and `answers`.
    - Wires `run <pack.gtpack> --answers <file>` as an alias for the same HTTP startup path.
    - Wires `mcp-tools <pack.gtpack>` to resolved `greentic.sorx.mcp-tools.v1` metadata from `assets/sorla/mcp-tools.json`.
    - Wires `mcp start <pack.gtpack> --answers <file>` to answer validation/runtime config construction and emits an adapter-only MCP runtime plan with resolved tools.
    - Wires `routes <pack.gtpack>` to generated route metadata from `agent-gateway.json`; `routes --deployment <deployment-id>` loads the local deployment registry, verifies the artifact digest, and emits deployment-scoped paths.
    - Wires `deployments list/inspect/create/validate/activate --private/promote/rollback/retire-old/public-routes/promotion-status/retire` to the local JSON deployment registry.
    - Wires `aliases set/list` to mutable alias pointers scoped by tenant and SOR name.
    - Wires `webhook verify-fixture` and `webhook replay --fixture --signature` to the signed GitHub/GHCR publish handler using a fake OCI resolver.
    - Wires `validate <pack.gtpack> --answers <file>` to a declarative validation-suite runner with `doctor`, artifact, route generation, provider contract, endpoint, negative endpoint, audit, idempotency, and policy-denial test kinds.
    - Wires `validation report <deployment-id>` to read stored registry validation reports once deployment-backed validation persistence is populated by later flows.
    - Accepts `--json` on `doctor`, `inspect`, `routes`, and `start` schema/dry-run/emit-answer command shapes for machine-readable compatibility. These outputs are JSON today.
    - Provides a dependency-light local HTTP adapter with `GET /healthz`, `GET /readyz`, `GET /v1/sorx/routes`, `GET /v1/sorx/public-routes`, `GET /v1/sorx/tools`, `GET /v1/sorx/deployments/local/routes`, `GET /v1/sorx/deployments/local/promotion-status`, generated agent endpoint routes, and disabled-by-default mutating admin API guards.
    - Serves `/v1/sorx/tools` from resolved MCP tool metadata rather than an unrelated placeholder surface.
    - Configures stdout audit sinks for HTTP runtimes when startup answers request `audit.sink = "stdout"`.
    - Requires tenant and caller headers outside local HTTP mode.
    - Includes parser tests and binary smoke tests.
    - Includes a PR 09 landlord/tenant e2e integration test that builds a deterministic `.gtpack`, runs `doctor`, starts the local HTTP runtime, calls generated routes, verifies create/read/update/query behavior, idempotency, high-risk approval-required behavior, response events, and MCP tool listing.
    - cargo-binstall metadata is configured for GitHub Release archives named `greentic-sorx-<target>-v<version>`.
  - **Key dependencies / integration points:** Depends on `greentic-sorx-core` and `greentic-sorx-pack`; provider-backed direct, HTTP, and MCP-adapter runtime execution are wired locally. Memory is the real local/CI provider. FoundationDB is registered through a clear unavailable adapter until the sibling provider exposes a SORX-compatible store contract. The audited sibling `gtc` implementation does not currently expose a generic `gtc sorx` route. Full MCP server transport, mutating HTTP admin API storage/auth, real GHCR auth/download integration, and configured-provider validation execution remain deferred to later PRs.

- **Path:** `ci/local_check.sh`
  - **Role:** Single local developer check entrypoint.
  - **Key functionality:**
    - Runs `cargo fmt --all -- --check`.
    - Runs i18n validation/status checks when `tools/i18n.sh` is present.
    - Runs `cargo clippy --all-targets --all-features -- -D warnings`.
    - Runs `cargo test --all-features`.
    - Runs `cargo build --all-features`.
    - Runs `cargo doc --no-deps --all-features`.
    - Detects publishable workspace crates through `cargo metadata` in dependency order and runs package/publish dry-run checks where Cargo can resolve registry dependencies.
    - Defers registry verification for a dependent crate when its local workspace dependency is not yet present in crates.io.
  - **Key dependencies / integration points:** Requires Cargo and `python3` for metadata parsing.

- **Path:** `scripts/local_check.sh`
  - **Role:** PR 11 compatibility wrapper for local checks.
  - **Key functionality:**
    - Delegates to `ci/local_check.sh` from the repository root.
    - Provides the requested `./scripts/local_check.sh` entrypoint while preserving the existing Greentic-style CI script.

- **Path:** `.github/workflows/ci.yml`
  - **Role:** Pull request and main-branch CI workflow.
  - **Key functionality:**
    - Runs lint, tests, and package/publish dry-run checks on Ubuntu.
    - Uses Rust stable, rustfmt, clippy, and Cargo cache.
    - Cancels redundant runs by branch/ref.

- **Path:** `.github/workflows/e2e.yml`
  - **Role:** Manual end-to-end workflow.
  - **Key functionality:**
    - Provides `workflow_dispatch` inputs for `scenario` and `provider`.
    - Runs the landlord/tenant e2e script for the memory provider.
    - Keeps FoundationDB as an optional/manual provider path that verifies the fixture and reports the adapter gap instead of requiring a live FoundationDB service in CI.

- **Path:** `.github/workflows/release.yml`
  - **Role:** Release readiness verification workflow.
  - **Key functionality:**
    - Triggers manually and on `v*` tags.
    - Runs `scripts/local_check.sh` after installing Rust, cache support, cargo-binstall, and the Greentic i18n translator.
    - Does not publish packages; publishing remains in `publish.yml`.

- **Path:** `.github/workflows/publish.yml`
  - **Role:** crates.io release workflow.
  - **Key functionality:**
    - Triggers on `workflow_dispatch` and `v*` tags.
    - Extracts the Cargo package version and verifies tag names match `v<version>`.
    - Runs `ci/local_check.sh`.
    - Publishes publishable crates in dependency order with a dry run and bounded retry loop.
  - **Key dependencies / integration points:** Requires `CARGO_REGISTRY_TOKEN`; GHCR publishing is not enabled.

- **Path:** `.github/workflows/release-binaries.yml`
  - **Role:** GitHub Release binary artifact workflow for cargo-binstall.
  - **Key functionality:**
    - Triggers on `v*` tags and manual dispatch.
    - Uses the Greentic org reusable release-binaries workflow for `greentic-sorx`.
    - Produces release archives matching the package metadata in `Cargo.toml`.
  - **Key dependencies / integration points:** Does not publish to GHCR.

- **Path:** `CHANGELOG.md`
  - **Role:** Release change log.
  - **Key functionality:**
    - Tracks unreleased SORX work and provides the changelog anchor required by the PR 11 release readiness pass.

- **Path:** `.github/workflows/perf.yml`
  - **Role:** Lightweight performance/concurrency guard workflow.
  - **Key functionality:**
    - Runs normal tests, including performance guard tests.
    - Runs a Criterion benchmark smoke test with a small sample size.

- **Path:** `.github/workflows/nightly-coverage.yml`
  - **Role:** Scheduled and manual coverage policy workflow.
  - **Key functionality:**
    - Installs Rust, llvm coverage tools, cargo-binstall, `cargo-nextest`, `cargo-llvm-cov`, and `greentic-dev`.
    - Runs `greentic-dev coverage --policy-file coverage-policy.json`.
    - Uploads `target/coverage/coverage.json` as an artifact when present.

- **Path:** `coverage-policy.json`
  - **Role:** Coverage enforcement policy for `greentic-dev coverage`.
  - **Key functionality:**
    - Requires 60% global and per-file line coverage by default.
    - Excludes `src/main.rs` as a thin binary entrypoint.

- **Path:** `i18n/`
  - **Role:** CLI localization assets.
  - **Key functionality:**
    - `i18n/en.json` defines current scaffold CLI strings.
    - `i18n/locales.json` lists all 66 supported locales.
    - Locale JSON files have been generated for all supported languages and validate successfully.

- **Path:** `tools/i18n.sh`
  - **Role:** Greentic CLI i18n helper.
  - **Key functionality:**
    - Adapted from the Greentic component i18n helper.
    - Uses `greentic-i18n-translator`.
    - Translates all configured languages with `I18N_BATCH_SIZE` defaulting to 200.
    - Supports `translate`, `validate`, `status`, and `all`.

- **Path:** `benches/perf.rs`
  - **Role:** Removed root benchmark path.
  - **Key functionality:** Replaced by the CLI crate benchmark.

- **Path:** `crates/greentic-sorx-cli/benches/perf.rs`
  - **Role:** Criterion benchmark harness.
  - **Key functionality:**
    - Benchmarks parsing the `start <pack.gtpack> --schema` CLI shape.
  - **Key dependencies / integration points:** Placeholder-style benchmark until real SORX hot paths exist.

- **Path:** `crates/greentic-sorx-cli/tests/perf_scaling.rs`
  - **Role:** Concurrency scaling guard test.
  - **Key functionality:**
    - Runs a CPU-bound representative workload with 1, 4, and 8 threads.
    - Fails if multi-thread execution degrades beyond broad guard thresholds.

- **Path:** `crates/greentic-sorx-cli/tests/perf_timeout.rs`
  - **Role:** Hang/slowdown guard test.
  - **Key functionality:**
    - Ensures a small representative workload completes within two seconds.

- **Path:** `crates/greentic-sorx-cli/tests/landlord_tenant_e2e.rs`
  - **Role:** PR 09 product e2e scenario.
  - **Key functionality:**
    - Builds a deterministic landlord/tenant `.gtpack` fixture during test setup from checked-in JSON fixture assets.
    - Runs the `greentic-sorx` binary through `doctor`, `mcp-tools`, and `start --answers`.
    - Drives generated HTTP routes for landlord, property, unit, tenant, tenancy, payment, and maintenance operations.
    - Verifies route listing, health/readiness, record read-back, active-tenant query, tenant update, idempotency on repeated tenant/payment creates, high-risk approval-required behavior, and emitted response events.
    - Includes a checked FoundationDB answers fixture for manual expansion while real FoundationDB execution remains blocked on the provider adapter gap.

- **Path:** `scripts/e2e/run-landlord-tenant.sh`
  - **Role:** Developer entrypoint for the PR 09 e2e.
  - **Key functionality:**
    - Runs the memory-provider landlord/tenant e2e test.
    - For `--provider foundationdb`, verifies the manual fixture and reports that real execution is not automated until the SORX-compatible FoundationDB store adapter exists.

- **Path:** `.codex/PR-01-*.md` through `.codex/PR-15-*.md`
  - **Role:** Planned PR roadmap for SORX.
  - **Key functionality:**
    - PR 01: audit Greentic reuse points and scaffold real `greentic-sorx` CLI/core structure. Implemented in the current workspace.
    - PR 02: implement `.gtpack` loading, inspection, and doctor validation. Implemented in the current workspace.
    - PR 03: implement startup schemas, answers, and `greentic-qa` integration. Implemented in the current workspace as a local schema adapter with documented interactive QA gap.
    - PR 04: implement runtime core, operation router, provider traits, and memory provider. Implemented in the current workspace.
    - PR 05: implement HTTP endpoint runtime generated from `agent-gateway.json`. Implemented in the current workspace as a local HTTP adapter over the PR 04 runtime path.
    - PR 06: implement policy, approvals, idempotency, and audit events. Implemented in the current workspace.
    - PR 07: implement MCP tool runtime from `mcp-tools.json`. Implemented in the current workspace as a local adapter and metadata surface; full MCP server transport remains a documented follow-up.
    - PR 08: implement provider binding resolution and FoundationDB adapter boundary. Implemented in the current workspace as entity binding resolution, tenant/pack/version operation namespaces, local/test direct provider config support, memory namespace isolation, and a clear unavailable FoundationDB adapter.
    - PR 09: add landlord/tenant end-to-end scenario. Implemented in the current workspace as a memory-provider binary e2e with deterministic fixture generation and documented FoundationDB/manual gaps.
    - PR 10: integrate with `gtc` extension/start conventions. Implemented as direct CLI alignment, stable exit codes, machine-readable `--json` compatibility flags, an actual `gtc` audit documenting the missing generic `gtc sorx` route, and `.gtbundle` integration design.
    - PR 11: harden CI, docs, security, determinism, and release readiness. Implemented as manual E2E/release-readiness workflows, `scripts/local_check.sh`, user/security/observability/release docs, changelog, future integrity metadata parsing, and focused security/determinism tests.
    - PR 12: add concurrent version deployment registry. Implemented as core registry/domain/storage types, conflict checks, local JSON persistence, deployment/alias CLI commands, deployment-scoped route listing, docs, and disabled-by-default HTTP admin guards.
    - PR 13: add GHCR publish webhook to pending deployment flow. Implemented as signed webhook parsing/verification, trust/digest/replay checks, OCI resolver trait with fixture resolver, pending registry deployment creation, startup config shape, CLI fixture verification/replay commands, and docs.
    - PR 14: add pack-embedded validation suite execution. Implemented as suite schema checks, fixture JSON loading, validation runner, report shape, validate CLI, JUnit output, docs, and smoke coverage for happy path, negative endpoint, idempotency, audit, and recommended-failure behavior.
    - PR 15: add public endpoint promotion, rollout, rollback, and gate policies. Implemented as validation-report-gated private/public promotion, alias promotion, alias rollback audit, old-deployment retirement, public route diagnostics, promotion-status diagnostics, traffic split metadata, expanded webhook promotion policy values, CLI commands, docs, and focused tests. Mutating HTTP admin endpoints remain disabled until auth and registry storage are wired into the runtime.
  - **Key dependencies / integration points:** PR 01 through PR 15 are implemented in source code/docs. Later PR files remain specifications only; real FoundationDB execution, real GHCR auth/download behavior, schema migration, full MCP server transport, mutating HTTP admin API implementation, configured-provider validation execution, and actual `gtc sorx` support in `gtc` are not yet implemented.

- **Path:** `.codex/global_rules.md`
  - **Role:** Repository working rules for future Codex PR-style work.
  - **Key functionality:**
    - Requires refreshing this overview before and after PR work.
    - Requires running `ci/local_check.sh` at the end of work.
    - Requires checking existing Greentic crates/repos before introducing shared types or cross-cutting behavior.

- **Path:** `README.md`
  - **Role:** Repository, command surface, and CI/release documentation.
  - **Key functionality:**
    - Explains SORX as System of Record eXecutor.
    - Documents the `.gtpack` runtime boundary and relationship to `greentic-sorla` and future `gtc sorx` usage.
    - Documents the current CLI command surface, including real `doctor`/`inspect`, route listing, local HTTP runtime, MCP tool listing, and adapter-only MCP startup behavior.
    - Documents `bash ci/local_check.sh`.
    - Documents release tagging as `vX.Y.Z`.
    - Documents `CARGO_REGISTRY_TOKEN`.
    - Links to PR 11 through PR 15 getting-started, command, answers, provider binding, MCP, deployment, GHCR webhook, validation suite, security, observability, signing/versioning, and release docs.

- **Path:** `docs/audit/reuse-audit.md`
  - **Role:** PR 01 Greentic reuse audit.
  - **Key functionality:**
    - Summarizes existing Greentic reuse points for `gtc`, `greentic-pack`, QA, startup answers, telemetry, HTTP, MCP, providers, secrets/OAuth, audit/events, and CI.
    - Documents decisions to reuse Greentic conventions, defer later runtime integrations, temporarily use a minimal local `.gtpack` reader because no small stable shared runtime-reader API is currently available to this repo, and ship a local MCP adapter while deferring full MCP server transport alignment.

- **Path:** `docs/audit/qa-reuse-pr03.md`
  - **Role:** PR 03 `greentic-qa` reuse note.
  - **Key functionality:**
    - Documents the sibling `greentic-qa` audit.
    - Explains why PR 03 uses a small local JSON-schema-oriented adapter instead of a sibling path dependency.
    - Documents supported `AnswerSet` envelope compatibility and the remaining interactive prompting gap.

- **Path:** `docs/audit/gtc-integration-pr10.md`
  - **Role:** PR 10 `gtc` integration audit.
  - **Key functionality:**
    - Documents actual sibling `gtc` extension behavior: fixed passthrough subcommands, extension registry/descriptor wizard launching, setup/start handoff files, stdio forwarding, and child exit-code preservation.
    - Records that `gtc` does not currently support a generic `gtc sorx` or arbitrary `greentic-*` discovery route.
    - Defines SORX's current direct invocation path, expected future `gtc sorx` forwarding shape, and stable exit-code table.

- **Path:** `docs/design/sorx-gtbundle-integration.md`
  - **Role:** PR 10 bundle integration design.
  - **Key functionality:**
    - Describes what a future SORX `.gtbundle` should contain without making SORX a bundle assembler.
    - Covers app packs, provider packs, startup answers, service launch delegation, signing/digest checks, and rollback by pack version/digest.

- **Path:** `docs/getting-started.md`
  - **Role:** User quickstart.
  - **Key functionality:** Documents the minimal SoRLa pack, SORX doctor, schema, answer, dry-run, and landlord/tenant e2e flow.

- **Path:** `docs/commands.md`
  - **Role:** CLI command reference.
  - **Key functionality:** Lists the current command surface and stable exit-code table.

- **Path:** `docs/answers.md`
  - **Role:** Startup answer guide.
  - **Key functionality:** Documents raw and `greentic-qa`-style answer files, defaulting, dry-run/emit-answer commands, and secret/provider-config restrictions.

- **Path:** `docs/provider-bindings.md`
  - **Role:** Provider binding guide.
  - **Key functionality:** Documents provider answer shape, entity binding shape, derived binding defaults, memory provider status, and the FoundationDB adapter gap.

- **Path:** `docs/mcp.md`
  - **Role:** MCP metadata/runtime guide.
  - **Key functionality:** Documents MCP tool metadata loading, tool listing, adapter-only MCP start behavior, and the deferred full MCP transport.

- **Path:** `docs/deployments.md`
  - **Role:** Deployment registry guide.
  - **Key functionality:** Documents the PR 12/15 local JSON registry, deployment and alias commands, deployment-scoped route listing, validation-gated public promotion, rollback, old-deployment retirement, states, and HTTP diagnostics.

- **Path:** `docs/ghcr-webhooks.md`
  - **Role:** GHCR publish webhook guide.
  - **Key functionality:** Documents PR 13 GitHub/GHCR callback handling, startup config, fixture replay commands, fixture shape, and the fake-resolver boundary before real GHCR integration.

- **Path:** `docs/validation-suites.md`
  - **Role:** Pack-embedded validation suite guide.
  - **Key functionality:** Documents PR 14 validation assets, minimal suite shape, supported declarative test kinds, validate CLI usage, provider modes, report shape, public-readiness behavior, and no-arbitrary-code rule.

- **Path:** `docs/security.md`
  - **Role:** Security model.
  - **Key functionality:** Documents pack path safety, lock digest checks, startup secret/config rules, non-local HTTP context requirements, idempotency guidance, request body audit redaction, default risk policies, and future signing gaps.

- **Path:** `docs/observability.md`
  - **Role:** Structured event reference.
  - **Key functionality:** Documents current audit event fields and runtime event sequence plus planned deployment lifecycle event names.

- **Path:** `docs/future-signing-and-versioning.md`
  - **Role:** Future integrity/versioning design note.
  - **Key functionality:** Documents recognized digest/signature manifest fields and later registry, alias, validation, and rollback expectations.

- **Path:** `docs/release.md`
  - **Role:** Release readiness guide.
  - **Key functionality:** Documents crate/bin name, semantic versioning, install paths, tag shape, changelog, release workflows, future Greentic toolchain manifest inclusion, and GHCR publishing policy.

- **Path:** `LICENSE`
  - **Role:** MIT license file.

## 3. Work In Progress, TODOs, and Stubs

- **Location:** `crates/greentic-sorx-cli/src/lib.rs`
  - **Status:** HTTP runtime surface
  - **Short description:** `doctor`, `inspect`, `routes <pack.gtpack>`, `routes --deployment <deployment-id>`, `mcp-tools <pack.gtpack>`, `mcp start <pack.gtpack> --answers <file>`, `deployments`, `aliases`, `start` schema/answer validation paths, `start --answers` local HTTP server startup, and `run --answers` alias behavior are implemented. Metadata/dry-run command shapes accept `--json`, and process failures use stable exit codes. Deployment-backed routes use the local JSON registry for known deployment IDs and still return an empty compatibility view for the implicit `local` deployment. `mcp start` currently emits an adapter-only runtime plan; full MCP server transport remains future work.

- **Location:** `crates/greentic-sorx-core/src/lib.rs`
  - **Status:** Runtime core
  - **Short description:** Core startup answer normalization, defaults, validation, runtime config construction, dry-run plan generation, endpoint routing, provider traits, provider registry, in-memory provider, provider binding resolver, tenant/pack/version provider namespaces, FoundationDB unavailable adapter boundary, policy decisions, local approval brokers, audit sinks/events, idempotency scoping, MCP tool metadata loading, MCP adapter invocation, invocation source tracking, and provider-backed endpoint invocation exist.

- **Location:** `crates/greentic-sorx-pack/src/loader.rs`
  - **Status:** Partial/shared-API gap
  - **Short description:** Implements a minimal local `.gtpack` runtime reader and doctor subset, including MCP tool metadata validation. It should be revisited if `greentic-pack` exposes a stable reusable library API for pack reading, lock verification, and doctor/inspect output.

- **Location:** `crates/greentic-sorx-cli/benches/perf.rs`
  - **Status:** Placeholder benchmark
  - **Short description:** Benchmarks CLI parsing; it should be supplemented or replaced with real hot paths after SORX pack loading, routing, validation, or provider code exists.

- **Location:** `crates/greentic-sorx-cli/tests/perf_scaling.rs` and `crates/greentic-sorx-cli/tests/perf_timeout.rs`
  - **Status:** Placeholder guard tests
  - **Short description:** Tests use synthetic workloads because the repo has no real SORX critical paths yet.

- **Location:** `.codex/PR-15-public-endpoint-promotion-rollout.md`
  - **Status:** Implemented in source/docs
  - **Short description:** PR 15 public promotion and rollback gates are implemented for the local registry and CLI. Mutating HTTP admin endpoints remain disabled until admin auth and registry storage are available in the runtime.

## 4. Broken, Failing, or Conflicting Areas

- **Location:** Repository checks
  - **Evidence:** `bash ci/local_check.sh` completed successfully after the PR 15 public promotion/rollback gate work. It ran formatting, i18n validation/status, clippy, tests, build, docs, and packaging/publish dry-run checks. A sandboxed `./scripts/local_check.sh` attempt during PR 11 reached the landlord/tenant binary e2e and failed only because the sandbox denied loopback `TcpListener::bind`; the same check passed outside the sandbox.
  - **Likely cause / nature of issue:** No current build/test failure was observed.

- **Location:** Coverage policy
  - **Evidence:** `greentic-dev coverage` completed successfully on May 13, 2026 with workspace line coverage of 80.83%.
  - **Likely cause / nature of issue:** No current coverage policy failure was observed.

- **Location:** Lightweight benchmark smoke
  - **Evidence:** `cargo bench -p greentic-sorx --bench perf -- --sample-size 10` completed successfully; benchmark reported `cli_parse_start_schema` around 25-34 microseconds.
  - **Likely cause / nature of issue:** No benchmark harness failure was observed. Results are not meaningful for product performance until real hot paths replace the synthetic benchmark.

- **Location:** i18n validation
  - **Evidence:** `tools/i18n.sh validate` and `tools/i18n.sh status` completed successfully for 66 languages.
  - **Likely cause / nature of issue:** No current locale key drift was observed.

- **Location:** Release automation scope
  - **Evidence:** `.github/workflows/publish.yml` supports crates.io publishing, and `.github/workflows/release-binaries.yml` supports GitHub Release binary artifacts for cargo-binstall.
  - **Likely cause / nature of issue:** GHCR publishing is intentionally disabled. Future runtime work may add GHCR read/resolve behavior for OCI references without publishing from this repo.

- **Location:** First-time workspace crate publishing
  - **Evidence:** `ci/local_check.sh` dry-runs independent publishable workspace crates and defers registry verification for dependent workspace crates while their local dependencies are not yet present in crates.io.
  - **Likely cause / nature of issue:** Cargo requires path dependencies to be resolvable from the registry before preparing a dependent crate for upload. The release workflow publishes dependency crates first, then dry-runs/publishes dependents with retry logic.

## 5. Notes for Future Work

- The `.codex` PR roadmap through PR 15 is now represented in source/docs. Remaining follow-up work is cross-repo or integration depth: real FoundationDB execution, real GHCR auth/download, mutating HTTP admin API auth/storage, configured-provider validation execution, schema migration, full MCP server transport, and actual `gtc sorx` support in `gtc`.
- Ask `gtc` to add a generic `gtc sorx`/`greentic-*` discovery route or a dedicated SORX passthrough if direct `gtc sorx ...` invocation is required.
- Ask `greentic-sorla-providers` to expose a SORX-compatible FoundationDB store provider or shared store trait; see `docs/audit/provider-foundationdb-gap-pr08.md`.
- Revisit `greentic-sorx-pack` when `greentic-pack` exposes a stable shared runtime-reader API, so local manifest/lock/archive logic can be reduced.
- Wire future `gtc sorx` execution to the direct `greentic-sorx` command shape once `gtc` routing support is implemented or documented.
- Replace synthetic perf workloads with real hot paths once pack loading, doctor validation, route generation, provider execution, or deployment registry code exists.
- Wire i18n strings into the CLI output/help once command messages stabilize beyond the PR 01 scaffold.
- Add GHCR read/resolve support when implementing the deployment registry/webhook roadmap; keep GHCR publishing disabled unless that decision changes.
- Keep `.codex/repo_overview.md` refreshed before and after each future PR-style change, and run `ci/local_check.sh` at the end of work.
