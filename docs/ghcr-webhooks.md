# GHCR Publish Webhooks

PR13 adds a safe webhook handling path for GitHub/GHCR publish callbacks. The
handler verifies the GitHub HMAC signature, checks repository/workflow/OCI
allowlists, resolves the exact OCI digest through an `OciArtifactResolver`
trait, records the GitHub delivery ID to prevent replay, and creates a pending
deployment in the local registry.

It does not publish endpoints or promote aliases directly. A webhook outcome may
request validation work for `validate_then_private`,
`validate_then_public_preview`, or `validate_then_public_alias`; PR15 promotion
still happens through the same registry lifecycle used by the CLI.

Startup answer config:

```json
{
  "ghcr_webhook": {
    "enabled": true,
    "bind": "127.0.0.1:8081",
    "public_path": "/v1/sorx/webhooks/github/ghcr-published",
    "signature_secret_ref": "secret://sorx/github/webhook-secret",
    "allowed_repositories": [
      "greenticai/greentic-sorla",
      "greenticai/greentic-sorla-providers"
    ],
    "allowed_oci_prefixes": [
      "ghcr.io/greenticai/sorla/",
      "ghcr.io/greenticai/sorla-providers/"
    ],
    "allowed_workflows": [
      "publish-gtpack.yml",
      "publish.yml"
    ],
    "default_promotion_policy": "validate_then_private"
  }
}
```

Fixture commands:

```bash
greentic-sorx webhook verify-fixture fixtures/github-ghcr-published.json
greentic-sorx --registry /tmp/sorx-registry.json webhook replay \
  --fixture fixtures/github-ghcr-published.json \
  --signature sha256:<hmac>
```

Fixture shape:

```json
{
  "event": "repository_dispatch",
  "delivery": "delivery-1",
  "secret": "test-only-secret",
  "resolved_digest": "sha256:abc123",
  "payload": {
    "repository": "greenticai/greentic-sorla",
    "workflow": "publish-gtpack.yml",
    "conclusion": "success",
    "artifact_kind": "sorla-gtpack",
    "oci_ref": "oci://ghcr.io/greenticai/sorla/landlord-tenant-sor:1.1.0",
    "digest": "sha256:abc123",
    "pack_name": "landlord-tenant-sor",
    "pack_version": "1.1.0",
    "tenant_id": "acme",
    "sor_name": "landlord-tenant",
    "environment": "staging",
    "api_version_label": "v1.1",
    "promotion_policy": "validate_then_private"
  }
}
```

The fixture `secret` field is only for local tests. Runtime startup answers must
use `signature_secret_ref`; raw webhook secrets should not be embedded.
