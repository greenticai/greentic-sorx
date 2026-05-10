# Future Signing and Versioning

Current SORX pack loading verifies `pack.lock.cbor` entry sizes and SHA-256
digests when a lock file is present. It also parses future manifest integrity
fields:

```json
{
  "integrity": {
    "digest": "sha256:...",
    "signature": "...",
    "signature_ref": "sigstore:..."
  }
}
```

These fields are recognized but not enforced yet.

Future deployment registry work should:

- Require exact pack digests for promoted deployments.
- Verify signatures before public exposure.
- Store multiple pack versions concurrently.
- Promote aliases such as `preview`, `stable`, and `latest` only after
  validation gates pass.
- Preserve rollback metadata by pack name, version, digest, and deployment ID.
