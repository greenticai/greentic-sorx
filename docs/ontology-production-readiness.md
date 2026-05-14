# Ontology Production Readiness

SORX treats ontology-enabled packs as untrusted runtime inputs. Production
readiness for this repo means validating those inputs, producing stable runtime
plans, and blocking public exposure until the pack has passed the configured
validation gates.

Required SORX checks:

- `greentic-sorx doctor --json` validates pack shape, ontology graph assets,
  retrieval bindings, startup schema, and embedded validation-suite metadata.
- `greentic-sorx start --dry-run --json` validates startup answers, normalizes
  defaults, and reports provider compatibility before runtime startup.
- `greentic-sorx validate --json` runs the embedded validation suite and emits
  `greentic.sorx.validation-report.v1`.
- Public promotion uses validation reports and ontology public-exposure gates
  before route publication.
- Provider capabilities are normalized into stable order before compatibility
  checks and startup-plan output.

The local production-readiness entry point is:

```bash
bash ci/local_check.sh
```

That check covers formatting, release metadata, clippy, tests, build, docs, and
packaging dry runs where the workspace can perform them locally.
