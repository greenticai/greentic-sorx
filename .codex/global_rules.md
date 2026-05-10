GLOBAL RULE - REPO OVERVIEW, CI, AND REUSE OF GREENTIC REPOS

For THIS REPOSITORY, you must ALWAYS:

1. Maintain `.codex/repo_overview.md` using the "Repo Overview Maintenance" routine BEFORE starting any new PR and AFTER finishing it.
2. Run `ci/local_check.sh` at the end of your work and ensure it passes, or explain precisely why it cannot be made to pass as part of this PR.
3. Prefer using existing Greentic repos/crates (interfaces, types, secrets, oauth, messaging, events, etc.) instead of reinventing types, interfaces, or behaviour locally.

Treat these as built-in prerequisites and finalisation steps for ALL work in this repo.

---

### Workflow for EVERY PR

Whenever implementing a change, feature, refactor, or bugfix (PR-style work), follow this workflow:

1. PRE-PR SYNC (MANDATORY)
   - Check out the target branch for this work (usually the default/main branch or the branch specified by the user).
   - Run the "Repo Overview Maintenance" routine:
     - Fully refresh `.codex/repo_overview.md` so it accurately reflects the current state of the repo before making any changes.
   - Show the updated `.codex/repo_overview.md` if it changed in a meaningful way.

2. IMPLEMENT THE PR
   - Apply the requested changes (code, tests, docs, configs, etc.).
   - Greentic reuse-first policy:
     - Before adding new core types, interfaces, or cross-cutting functionality, check whether they already exist in other Greentic repos, for example:
       - `greentic-interfaces`
       - `greentic-types`
       - `greentic-secrets`
       - `greentic-oauth`
       - `greentic-messaging`
       - `greentic-events`
       - and other existing shared crates as relevant
     - If a suitable type or interface exists, use it instead of redefining it locally.
     - Do not fork or duplicate cross-repo models unless there is a clear, documented reason.
     - Only introduce new shared concepts when there is no existing crate that fits; if so, clearly mention this in the PR summary.
   - Run the appropriate build/test commands while working, and fix issues related to the changes.

3. POST-PR SYNC (MANDATORY)
   - Re-run the "Repo Overview Maintenance" routine against the updated codebase:
     - Update `.codex/repo_overview.md` to reflect new functionality, resolved or added TODO/WIP/stub entries, and current failures.
   - Run the repo CI wrapper:
     - Execute `ci/local_check.sh` from the repo root.
     - If it fails due to the changes, fix the issues until it passes.
     - If it fails for reasons outside the scope of the changes, capture the failing steps and key error messages and document them clearly.
   - Ensure `.codex/repo_overview.md` is consistent and up to date.
   - In the final PR summary, explicitly mention:
     - That the repo overview was refreshed.
     - That `ci/local_check.sh` was run and its outcome.

---

### Behavioural Rules

- Do not ask for permission to run the Repo Overview Maintenance routine, run `ci/local_check.sh`, or reuse existing Greentic crates.
- Never leave `.codex/repo_overview.md` in a partially updated or obviously inconsistent state.
- Never introduce new core types or interfaces that duplicate what exists in shared Greentic crates without a strong, documented justification.
- If the build/test/CI commands are unclear and cannot be inferred from repo files, ask a concise question; otherwise, proceed autonomously.

---

The "Repo Overview Maintenance" routine is defined in `.codex/repo_overview_task.md`. Follow it exactly whenever instructed above.
