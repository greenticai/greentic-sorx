# PR 03 — Sorx Startup Schema, Answers, and `greentic-qa` Integration

## Goal

Make Sorx startup consistent with existing Greentic patterns such as:

```bash
gtc start --schema
gtc start --answers answers.json
```

Sorx should support answer-driven startup with schemas and interactive question asking for missing answers through `greentic-qa`.

## Required CLI

Implement:

```bash
greentic-sorx start landlord.gtpack --schema
greentic-sorx start landlord.gtpack --answers sorx.answers.json
greentic-sorx start landlord.gtpack --answers partial.answers.json
greentic-sorx start landlord.gtpack --answers sorx.answers.json --dry-run
greentic-sorx start landlord.gtpack --answers partial.answers.json --emit-answers
```

Future `gtc` extension form should work similarly:

```bash
gtc sorx start landlord.gtpack --schema
gtc sorx start landlord.gtpack --answers sorx.answers.json
```

## Behaviour

`--schema`:

- loads the `.gtpack`
- validates it enough to locate `assets/sorx/start.schema.json`
- emits the startup schema to stdout
- does not ask questions
- does not start runtime

`--answers answers.json`:

- loads startup schema from pack
- loads provided answers
- validates answers
- if answers are complete: builds normalized runtime config
- if answers are incomplete and interactive: asks missing questions via `greentic-qa`
- if answers are incomplete and non-interactive: fails with a clear list of missing paths
- does not embed secrets in output

`--dry-run`:

- validates pack and answers
- produces a startup plan
- does not start server
- does not connect to providers

`--emit-answers`:

- writes normalized full answers to stdout or file
- stable formatting
- useful for CI

## Reuse `greentic-qa`

Audit and reuse existing `greentic-qa` structures for:

- schema-driven questions
- answer envelopes
- missing-answer detection
- locale/i18n
- validation
- normalized answer output

If `greentic-qa` does not expose reusable APIs, implement an adapter and document the gap.

## Startup schema shape

Sorx packs should contain a schema similar to:

```json
{
  "schema": "greentic.sorx.start.schema.v1",
  "title": "Sorx startup answers",
  "type": "object",
  "required": ["tenant", "server", "providers", "policy", "audit"],
  "properties": {
    "tenant": {
      "type": "object",
      "required": ["tenant_id", "environment"],
      "properties": {
        "tenant_id": { "type": "string" },
        "environment": { "type": "string", "default": "local" }
      }
    },
    "server": {
      "type": "object",
      "required": ["bind", "public_base_url"],
      "properties": {
        "bind": { "type": "string", "default": "127.0.0.1:8787" },
        "public_base_url": { "type": "string" }
      }
    },
    "mcp": {
      "type": "object",
      "properties": {
        "enabled": { "type": "boolean", "default": false },
        "bind": { "type": "string", "default": "127.0.0.1:8790" }
      }
    },
    "providers": {
      "type": "object",
      "required": ["store"],
      "properties": {
        "store": {
          "type": "object",
          "required": ["kind", "config_ref"],
          "properties": {
            "kind": { "type": "string", "enum": ["memory", "foundationdb"] },
            "config_ref": { "type": "string" }
          }
        }
      }
    },
    "policy": {
      "type": "object",
      "required": ["approvals"],
      "properties": {
        "approvals": {
          "type": "object",
          "properties": {
            "low": { "type": "string", "enum": ["auto", "require_approval", "deny"], "default": "auto" },
            "medium": { "type": "string", "enum": ["auto", "require_approval", "deny"], "default": "auto" },
            "high": { "type": "string", "enum": ["auto", "require_approval", "deny"], "default": "require_approval" },
            "critical": { "type": "string", "enum": ["auto", "require_approval", "deny"], "default": "deny" }
          }
        }
      }
    },
    "audit": {
      "type": "object",
      "properties": {
        "sink": { "type": "string", "enum": ["stdout", "file", "disabled"], "default": "stdout" }
      }
    }
  }
}
```

Use the actual generated schema from the `.gtpack` where present. This is only a fallback/example.

## Runtime config generation

Add:

```rust
pub struct SorxStartAnswers {
    pub tenant: TenantAnswers,
    pub server: ServerAnswers,
    pub mcp: McpAnswers,
    pub providers: ProviderAnswers,
    pub policy: PolicyAnswers,
    pub audit: AuditAnswers,
}

pub struct SorxRuntimeConfig {
    pub tenant_id: String,
    pub environment: String,
    pub server: ServerConfig,
    pub mcp: McpConfig,
    pub providers: ProviderConfigMap,
    pub bindings: BindingConfig,
    pub policy: PolicyConfig,
    pub audit: AuditConfig,
}
```

The runtime config should be constructed from:

- pack metadata
- SorLa/Sorx assets
- startup answers
- optional runtime templates

No secrets should be embedded in `.gtpack`.

## Startup plan

For `--dry-run`, emit:

```json
{
  "schema": "greentic.sorx.start.plan.v1",
  "pack": {
    "name": "landlord-tenant-sor",
    "version": "0.1.0"
  },
  "server": {
    "bind": "127.0.0.1:8787"
  },
  "mcp": {
    "enabled": true
  },
  "providers": [
    {
      "id": "store",
      "kind": "foundationdb",
      "config_ref": "providers.foundationdb.local"
    }
  ],
  "policy": {
    "high": "require_approval"
  }
}
```

## Tests

Add tests for:

- `--schema` emits pack schema
- full answers validate
- missing answers are detected
- non-interactive missing answers fail clearly
- defaults are applied
- normalized answers are stable
- dry-run emits stable startup plan
- secret-like values are rejected or warned depending policy
- invalid provider kind fails validation
- invalid approval mode fails validation

## Acceptance criteria

- Sorx supports `start --schema`.
- Sorx supports `start --answers`.
- Missing answers follow Greentic QA behaviour.
- Non-interactive mode fails without prompting.
- Normalized answers and dry-run plan are deterministic.
- No runtime starts in this PR unless already trivial.
- Tests cover full, partial, and invalid answers.

## Codex working style

Complete as much as possible in one pass. Reuse `greentic-qa` if possible. If not possible, implement a small adapter and document future reuse work.


## v2 additions — deployment and exposure answers

Add answer fields that can later be consumed by PR 12–PR 15:

```yaml
deployment:
  tenant_id: string
  sor_name: string
  environment: local | dev | staging | production
  deployment_mode: local_single | versioned_registry
  api_version_label: string
  base_path: string
  alias: optional string

exposure:
  default_visibility: private | internal | public
  require_validation_suite: boolean
  auto_promote_on_validation_pass: boolean
  public_aliases_allowed:
    - stable
    - latest
    - preview

ghcr:
  enable_publish_webhook: boolean
  allowed_repositories:
    - ghcr.io/greenticai/sorla-packages/*
  require_exact_digest: boolean
```

Defaults for local development should remain simple. Production answers must require explicit tenant, environment, public exposure policy, and provider binding references.
