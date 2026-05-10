# PR 10 `gtc` Integration Audit

SORX audited the sibling `../greentic` `gtc` implementation before assuming a
`gtc sorx` route exists.

## Current `gtc` Behavior

- `gtc` has fixed passthrough subcommands for `dev`, `op`, `wizard`, and
  `setup`.
- Companion binaries are resolved from env overrides, sibling release binaries,
  workspace-local targets, or Cargo's bin directory.
- Passthrough preserves stdio and returns the child process exit code.
- `gtc wizard --extensions` uses an extension registry and descriptor files to
  launch extension wizard binaries.
- `gtc setup --extension-setup-handoff` and
  `gtc start --extension-start-handoff` consume normalized handoff JSON
  documents.
- There is no generic `gtc <name>` discovery route for arbitrary
  `greentic-<name>` binaries today, so `gtc sorx ...` cannot be tested or
  claimed without a `gtc` change.

## SORX Compatibility

Direct invocation is the supported path today:

```bash
greentic-sorx start landlord.gtpack --schema
greentic-sorx start landlord.gtpack --answers answers.json
```

The command shape is intentionally compatible with Greentic answer-driven start
semantics:

- `--schema`
- `--answers`
- `--emit-answers`
- `--dry-run`
- `--non-interactive`
- `--json` for machine-readable output on metadata/dry-run commands

When `gtc` adds a `sorx` passthrough or generic `greentic-*` discovery route,
the expected forwarded form is:

```bash
greentic-sorx start landlord.gtpack --schema
greentic-sorx start landlord.gtpack --answers answers.json
```

## Exit Codes

SORX uses these stable process exit codes:

| Code | Meaning |
| --- | --- |
| 0 | success |
| 1 | generic error |
| 2 | invalid CLI usage |
| 3 | pack validation failed |
| 4 | answers validation failed |
| 5 | provider resolution failed |
| 6 | runtime startup failed |
| 7 | policy denied during dry-run |

## Manual `gtc` Test Once Available

After `gtc` gains a `sorx` route:

```bash
gtc sorx start landlord.gtpack --schema
gtc sorx start landlord.gtpack --answers answers.json --dry-run --json
```

Expected behavior: stdout is stable JSON for schema/dry-run, stderr carries
human diagnostics, and the exit code matches the SORX table above.
