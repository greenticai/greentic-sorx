# PR 13 — GHCR Publish Webhook to Pending SORX Deployment

## Goal

Allow SORX to receive a signed webhook after a GitHub workflow successfully publishes a SORLA `.gtpack` or provider artifact to GHCR, then create a pending versioned deployment using the exact OCI reference and digest.

The webhook must not make endpoints public by itself. It may only install/resolve the artifact and create a `pending` or `validating` deployment depending on policy.

## Trigger model

The intended upstream flow is:

```text
GitHub Actions builds SORLA .gtpack
  -> publishes artifact to GHCR as OCI artifact
  -> workflow completes successfully
  -> GitHub sends webhook to SORX deployer endpoint
  -> SORX verifies signature and workflow conclusion
  -> SORX resolves exact GHCR digest
  -> SORX creates pending deployment
  -> SORX runs pack doctor and validation suite
  -> SORX may promote private/public based on policy
```

Support either:

1. GitHub `workflow_run` webhook for successful publish workflows.
2. GitHub `repository_dispatch` or `workflow_dispatch` callback carrying explicit artifact metadata.

Do not rely on the mutable tag alone. Always resolve and store the exact digest.

## Webhook endpoint

Add an admin-only webhook server route:

```text
POST /v1/sorx/webhooks/github/ghcr-published
```

Require:

```text
X-Hub-Signature-256
X-GitHub-Event
X-GitHub-Delivery
```

Validate payload signature using a configured secret reference, never a raw secret embedded in answers.

## Accepted payload metadata

The handler must derive or accept:

```json
{
  "repository": "greenticai/greentic-sorla",
  "workflow": "publish-gtpack.yml",
  "conclusion": "success",
  "artifact_kind": "sorla-gtpack",
  "oci_ref": "oci://ghcr.io/greenticai/sorla/landlord-tenant-sor:1.1.0",
  "digest": "sha256:...",
  "pack_name": "landlord-tenant-sor",
  "pack_version": "1.1.0",
  "tenant_id": "acme",
  "sor_name": "landlord-tenant",
  "environment": "staging",
  "api_version_label": "v1.1",
  "promotion_policy": "validate_then_private"
}
```

For `workflow_run`, the OCI metadata may need to be fetched from a release asset, workflow artifact, GHCR manifest annotation, or repository-dispatch payload. Implement the parser so it is testable with fixtures.

## OCI/GHCR resolver

Add an `OciArtifactResolver` abstraction:

```rust
trait OciArtifactResolver {
    async fn resolve(&self, reference: &OciReference) -> Result<ResolvedOciArtifact>;
    async fn download_gtpack(&self, resolved: &ResolvedOciArtifact) -> Result<PackBytes>;
}
```

The resolved artifact must include:

```text
original_ref
resolved_digest
media_type
size
annotations
local_cache_path
```

Initial implementation may use an existing Greentic distributor/OCI helper if available. Otherwise keep a minimal implementation behind a trait and test with a fake resolver.

## Security requirements

- Verify GitHub webhook HMAC signature.
- Reject non-successful workflow conclusions.
- Reject untrusted repositories, owners, package prefixes, or environments.
- Reject mutable tags unless `require_exact_digest=false` for local/dev only.
- Store the GitHub delivery ID to prevent replay.
- Do not log secrets or authorization headers.
- Do not start public endpoints from an unvalidated webhook.

## Config/answers

Add configuration:

```yaml
ghcr_webhook:
  enabled: true
  bind: 127.0.0.1:8081
  public_path: /v1/sorx/webhooks/github/ghcr-published
  signature_secret_ref: secret://sorx/github/webhook-secret
  allowed_repositories:
    - greenticai/greentic-sorla
    - greenticai/greentic-sorla-providers
  allowed_oci_prefixes:
    - ghcr.io/greenticai/sorla/
    - ghcr.io/greenticai/sorla-providers/
  allowed_workflows:
    - publish-gtpack.yml
    - publish.yml
  default_promotion_policy: validate_then_private
```

## CLI

Add:

```bash
greentic-sorx webhook verify-fixture fixtures/github-ghcr-published.json

greentic-sorx webhook replay   --fixture fixtures/github-ghcr-published.json   --signature <test-signature>
```

## Tests

Add fixtures for:

- successful `workflow_run` publish event
- failed workflow event rejected
- unsigned event rejected
- bad HMAC rejected
- untrusted repository rejected
- untrusted OCI prefix rejected
- replay delivery ID rejected
- valid event creates pending deployment
- valid event with policy `validate_then_private` triggers validation job but not public exposure

Use fake GHCR/OCI resolver in tests.

## Acceptance criteria

- A successful GHCR publish webhook creates a pending deployment with exact digest.
- No endpoint is made public from webhook alone.
- Webhook security and replay protection are covered by tests.
- The implementation supports future distributor-client integration without changing public SORX deployment concepts.

## Codex working style

Do not attempt to implement full GHCR auth if an existing Greentic OCI/distributor helper is not available. Use an interface and fake resolver so the PR can land safely, then wire the real resolver in a follow-up.
