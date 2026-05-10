# Getting Started

SORX runs a SoRLa `.gtpack` as a system-of-record runtime.

Minimal flow:

```bash
greentic-sorla pack examples/landlord.sorla --out landlord.gtpack
greentic-sorx doctor landlord.gtpack
greentic-sorx start landlord.gtpack --schema > sorx.schema.json
greentic-sorx start landlord.gtpack --answers examples/landlord.answers.json
```

Useful local dry runs:

```bash
greentic-sorx inspect landlord.gtpack
greentic-sorx routes landlord.gtpack --json
greentic-sorx mcp-tools landlord.gtpack
greentic-sorx start landlord.gtpack --answers examples/landlord.answers.json --dry-run --json
greentic-sorx start landlord.gtpack --answers examples/landlord.answers.json --emit-answers
```

For the checked-in landlord/tenant scenario:

```bash
bash scripts/e2e/run-landlord-tenant.sh --provider memory
```
