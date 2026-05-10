# PR 03 Greentic QA Reuse Note

PR 03 audited the sibling `greentic-qa` repository before adding startup answer
handling locally.

`greentic-qa` exposes reusable Rust crates such as `qa-spec` and
`greentic-qa-lib` for FormSpec-driven wizard flows, `AnswerSet` envelopes,
answer normalization, validation, render payloads, and interactive runner
planning. Those crates live in the `greentic-qa` workspace and currently carry
their own workspace dependency graph and Rust version policy.

SORX startup packs currently embed `assets/sorx/start.schema.json` as a JSON
schema-like runtime contract rather than a full `qa-spec` FormSpec. To avoid
adding a path dependency to a sibling checkout or a larger wizard dependency
surface before the pack contract settles, PR 03 uses a small local adapter:

- raw SORX startup answer objects are accepted directly;
- `greentic-qa`-style `AnswerSet` JSON envelopes with `form_id`,
  `spec_version`, and `answers` are accepted by unwrapping the `answers` field;
- schema defaults, missing required paths, enum/type checks, normalized output,
  and deterministic startup plans are handled in `greentic-sorx-core`;
- inline secret-like answers are rejected unless they are references.

Interactive prompting remains the explicit gap. Non-interactive missing-answer
behavior is deterministic and reports missing paths. A later PR can replace or
extend this adapter with direct `greentic-qa-lib` wizard execution once SoRLa
emits a compatible FormSpec or the shared QA crates are published with a stable
runtime-reader API suitable for SORX.
