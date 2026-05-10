# Startup Answers

`greentic-sorx start <pack.gtpack> --schema` emits the startup schema embedded
in the pack. Answer files may use the raw SORX answer object or a
`greentic-qa`-style envelope:

```json
{
  "form_id": "greentic.sorx.start",
  "spec_version": "0.1.0",
  "answers": {
    "tenant": { "tenant_id": "tenant-a" }
  }
}
```

SORX applies schema defaults and validates required values. Use:

```bash
greentic-sorx start landlord.gtpack --answers landlord.answers.json --emit-answers
greentic-sorx start landlord.gtpack --answers landlord.answers.json --dry-run --json
```

Security rules:

- Inline secret-like values are rejected unless they are references.
- Direct provider `config` is allowed only in `local` or `test`.
- Use `config_ref` for shared, staging, and production environments.
