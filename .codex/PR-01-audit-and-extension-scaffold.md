# PR 01 — Audit Existing Greentic Reuse Points and Scaffold `greentic-sorx`

## Goal

Create the initial `greentic-sorx` repository/crate scaffold as a `gtc` extension-compatible executable, but first audit what already exists in the Greentic codebase so Sorx does not reinvent pack loading, QA, startup, logging, MCP, HTTP, provider, or extension mechanisms.

Sorx means **System of Record eXecutor**.

It consumes SoRLa `.gtpack` artifacts and runs them. It must not consume loose `./dist` folders as the primary runtime contract.

## Important boundary

`greentic-sorla` owns:

- authoring language
- parser and validation
- canonical IR
- deterministic handoff artifacts
- `.gtpack` production by reusing `greentic-pack`

`greentic-sorx` owns:

- `.gtpack` runtime loading
- runtime validation
- startup answers
- HTTP and MCP runtime
- provider binding
- policy/approval enforcement
- audit/logging
- local/e2e execution

Do not move runtime responsibilities into `greentic-sorla`.

## First task: repository audit

Before creating new modules, audit the existing Greentic repos available locally or in the workspace.

Check for reusable implementations of:

- `gtc` extension discovery
- direct `greentic-*` executable invocation
- `greentic-pack` `.gtpack` read/write/doctor APIs
- pack manifest and lock parsing
- asset lookup inside `.gtpack`
- canonical CBOR helpers
- `greentic-qa` question/schema/answers flow
- `gtc start --schema` / `--answers` patterns
- config manager or answer envelope patterns
- logging/tracing conventions
- HTTP server conventions
- MCP server/tool conventions
- provider registration patterns
- secrets/OAuth references
- audit/event sink conventions
- CI/local check conventions

Produce a short `docs/audit/reuse-audit.md` summarising:

```text
Capability | Existing repo/crate/module | Reuse decision | Notes
```

Do not assume repos exist. If a capability does not exist, say so and create only a small local abstraction.

## Scaffold

Create a new repo or workspace package according to existing Greentic conventions:

```text
greentic-sorx/
  Cargo.toml
  README.md
  crates/
    greentic-sorx-cli/
    greentic-sorx-core/
```

Start with a minimal split only. Do not over-split until needed.

The binary should be named:

```text
greentic-sorx
```

It must support both direct execution:

```bash
greentic-sorx --help
```

and future `gtc` extension style:

```bash
gtc sorx --help
```

If extension discovery is already implemented elsewhere, integrate with it. If not, document the expected convention and keep the executable compatible.

## Initial CLI

Implement placeholder commands:

```bash
greentic-sorx doctor <pack.gtpack>
greentic-sorx inspect <pack.gtpack>
greentic-sorx routes <pack.gtpack>
greentic-sorx start <pack.gtpack> --schema
greentic-sorx start <pack.gtpack> --answers answers.json
greentic-sorx run <pack.gtpack> --answers answers.json
```

For this PR, commands may return `not yet implemented` except `--help` and version output, but CLI parsing and command structure should be real.

Prefer `start` as the primary command name for consistency with `gtc start`. `run` may be an alias if useful.

## Data structures

Add initial core types:

```rust
pub struct SorxVersion {
    pub schema: String,
    pub version: String,
}

pub struct SorxCommandContext {
    pub working_dir: PathBuf,
    pub non_interactive: bool,
}

pub enum SorxCommand {
    Doctor,
    Inspect,
    Routes,
    Start,
    Run,
}
```

Add placeholders for later:

```rust
pub struct LoadedSorlaPack;
pub struct SorxStartAnswers;
pub struct SorxRuntimeConfig;
```

## Documentation

Add `README.md` explaining:

- what Sorx is
- why input is `.gtpack`
- what Sorx does not do
- relationship with `greentic-sorla`
- relationship with `gtc`
- expected future command flow

Example:

```bash
greentic-sorla pack landlord.sorla --out landlord.gtpack
gtc sorx start landlord.gtpack --schema
gtc sorx start landlord.gtpack --answers landlord.sorx.answers.json
```

## Tests

Add tests for:

- CLI parses each command
- help output contains `doctor`, `inspect`, `routes`, `start`
- version output works
- invalid command fails clearly
- `.gtpack` positional argument is required for pack commands

## Acceptance criteria

- `greentic-sorx` binary builds.
- CLI has the expected command surface.
- Audit document exists.
- No runtime server is implemented yet.
- No loose `./dist` runtime contract is introduced.
- Missing Greentic reuse points are documented rather than invented silently.

## Codex working style

Complete as much as possible in one pass. Routine scaffolding, docs, tests, and small refactors are pre-approved. Do not repeatedly ask permission. Stop only for destructive actions, credentials, or publishing.
