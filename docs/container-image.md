# Container image

`ghcr.io/greenticai/greentic-sorx` packages the `greentic-sorx` binary for
Kubernetes and Cloud Run env-packs, which run one sorx service per System of
Record.

## What is in it

- Base: `gcr.io/distroless/cc-debian13:nonroot`, pinned by digest in
  `docker/Dockerfile`. Debian 13 because the release binaries are built on
  ubuntu-24.04 and need glibc 2.39; Debian 12's glibc 2.36 cannot load them.
  Distroless `cc` adds `libgcc_s` and CA certificates; there is no shell and no
  package manager.
- One file of ours: `/usr/local/bin/greentic-sorx`, the entrypoint.
- Runs as uid/gid `65532` (`nonroot`), `HOME=/home/nonroot`, working directory
  `/home/nonroot`.
- Platforms: `linux/amd64`, `linux/arm64`.
- Default command: `--help`.

The binary is not compiled during the image build. It is the exact release
artifact Dev Publish attached to the GitHub release, verified against its
`.sha256` sidecar and checked for the right ELF architecture
(`docker/fetch-release-binaries.sh`).

## Tags

Published by `.github/workflows/container-image.yml` on every push to
`develop`:

| tag | meaning |
|---|---|
| `develop` | moves with every develop push |
| `<version>` | the stamped dev version of that push's release, e.g. `0.2.34194313992` for release `v0.2.34194313992`; immutable in practice |

Pin a deployment to `<version>` (or a digest), not to `develop`.

The binary reports `--version` from its `Cargo.toml` (currently
`greentic-sorx 0.2.0-dev.0`), not the stamped release version; the image's
`org.opencontainers.image.version` label and its tag carry the stamped one.

## How the workflow finds the binaries

The workflow runs on `push` to `develop` and waits (up to 90 minutes) for the
Dev Publish run of the same commit to attach its release. The release tag ends
in that run's id (`v0.2.<run id>` / `vX.Y.Z-dev.<run id>`), which is how the
two are matched. It waits on the release, not on Dev Publish's conclusion,
because Dev Publish's final crates.io step can fail after the release is
already complete.

`workflow_run` is not used: GitHub fires it only for workflow files on the
default branch (`main`), which carries neither Dev Publish nor this workflow.

## Running it

```bash
docker run --rm ghcr.io/greenticai/greentic-sorx:develop --version

docker run --rm \
  -v "$PWD/landlord.gtpack:/work/landlord.gtpack:ro" \
  -v "$PWD/landlord.answers.json:/work/answers.json:ro" \
  -p 8787:8787 \
  ghcr.io/greenticai/greentic-sorx:develop \
  start /work/landlord.gtpack --answers /work/answers.json --non-interactive
```

`start` (alias `run`) takes the pack path as a positional argument; see
[commands](commands.md) and [startup answers](answers.md).
`start <pack> --schema` prints the answers the pack accepts, and
`--dry-run --json` validates pack and answers without starting.

**Set `server.bind` to `0.0.0.0:<port>`.** Its default, `127.0.0.1:8787`, is
unreachable from outside the container. Set `server.public_base_url` to the
address callers use (the Service or Cloud Run URL). Minimal answers:

```json
{
  "tenant": { "tenant_id": "tenant-a", "environment": "production" },
  "server": {
    "bind": "0.0.0.0:8787",
    "public_base_url": "https://supplier-sor.example.com"
  }
}
```

## Store configuration

The store comes from the startup answers (`providers.store.kind` plus
`config_ref` / `config`) and from environment variables. A `postgres` store
kind that reads its connection URL from `SORX_POSTGRES_URL` is being added
separately; once it lands, pass the URL as an environment variable (from a
Kubernetes Secret or Cloud Run secret), never inside the answers file, which
rejects inline secret-like values outside `local`/`test`.

The container filesystem is ephemeral: a `memory` store, or anything written
under `/home/nonroot`, is lost when the pod or instance restarts.

## Building locally

```bash
docker/fetch-release-binaries.sh v0.2.34194313992 /tmp/sorx-context
docker buildx build --platform linux/amd64 -f docker/Dockerfile \
  -t greentic-sorx:local --load /tmp/sorx-context
docker run --rm greentic-sorx:local --version
```

## Operational notes

- **First publish is private.** A ghcr package created by `GITHUB_TOKEN` is
  private by default. After the first successful run, set
  `ghcr.io/greenticai/greentic-sorx` to public in the org's package settings,
  or every unauthenticated pull (Kubernetes or Cloud Run without a pull
  secret) fails while the workflow stays green.
- **Not every develop push gets an image.** Runs share one concurrency group
  and GitHub keeps only one pending run, so a burst of pushes builds the
  first and the last. `:develop` only ever moves forward.
- **A rerun of Dev Publish does not rebuild the image.** If Dev Publish failed
  and was rerun successfully, rerun this workflow's failed job too
  (`gh run rerun <id> --failed`).
- **The sha256 check proves integrity, not origin.** The digest is read from
  the release's own `.sha256` sidecar, so it catches a corrupted download, not
  a replaced release asset. Provenance attestation on the release build is the
  stronger check, and is not in place yet.
