#
# Runtime image for the `greentic-sorx` binary: ghcr.io/greenticai/greentic-sorx.
#
# Nothing is compiled here. The build context is prepared by
# .github/workflows/container-image.yml, which downloads the linux-gnu release
# archives Dev Publish attached to the GitHub release, verifies each against
# its .sha256 sidecar, and lays the binaries out as:
#
#   bin/amd64/greentic-sorx
#   bin/arm64/greentic-sorx
#
# BuildKit sets TARGETARCH per platform, so one Dockerfile serves both
# architectures, and because there is no RUN step no emulation (QEMU) is needed.
#
# Base: distroless cc on Debian 13 (trixie), NOT debian12. The release binaries
# are built on ubuntu-24.04 and require glibc 2.39 (their highest versioned
# symbol is GLIBC_2.39); debian12 ships glibc 2.36 and the binary would fail to
# load. trixie ships glibc 2.41. The `cc` flavour adds libgcc_s, which the
# binary links (NEEDED: libgcc_s.so.1, libm.so.6, libc.so.6).
#
# The digest is pinned on purpose; bump it deliberately (read the new one with
# `docker buildx imagetools inspect gcr.io/distroless/cc-debian13:nonroot`).
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:54df941ed0d06a1bd95ef5e0ce391fd8d9f94b64782dc9a60062727849ee3f97

ARG TARGETARCH
ARG VERSION=unknown
ARG REVISION=unknown

LABEL org.opencontainers.image.title="greentic-sorx" \
      org.opencontainers.image.description="Runs a SoRLa .gtpack as a Greentic System of Record" \
      org.opencontainers.image.source="https://github.com/greenticai/greentic-sorx" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

COPY --chmod=0755 bin/${TARGETARCH}/greentic-sorx /usr/local/bin/greentic-sorx

# distroless `nonroot` is uid/gid 65532 with HOME=/home/nonroot, which is where
# the CLI's default deployment registry resolves. Numeric so Kubernetes
# `runAsNonRoot` can verify it without resolving a name.
USER 65532:65532
WORKDIR /home/nonroot

# The HTTP runtime's conventional port. The runtime binds whatever
# `server.bind` in the startup answers says, and its default is 127.0.0.1:8787,
# which is unreachable from outside the container: answers must set
# `server.bind` to 0.0.0.0:<port>.
EXPOSE 8787

ENTRYPOINT ["/usr/local/bin/greentic-sorx"]
CMD ["--help"]
