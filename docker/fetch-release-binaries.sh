#!/usr/bin/env bash
# Download the linux-gnu `greentic-sorx` release archives for one GitHub release,
# verify each against its .sha256 sidecar, check the ELF architecture, and lay
# the binaries out as the build context docker/Dockerfile expects:
#
#   <out>/bin/amd64/greentic-sorx
#   <out>/bin/arm64/greentic-sorx
#
# Usage: docker/fetch-release-binaries.sh <release tag> <out dir>
# Needs: gh (authenticated; GH_TOKEN in CI), sha256sum, tar, file.
# REPO defaults to greenticai/greentic-sorx.
set -euo pipefail

tag="${1:?usage: fetch-release-binaries.sh <release tag> <out dir>}"
out="${2:?usage: fetch-release-binaries.sh <release tag> <out dir>}"
repo="${REPO:-greenticai/greentic-sorx}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# docker arch  rust target triple            `file` signature
targets=(
  "amd64 x86_64-unknown-linux-gnu x86-64"
  "arm64 aarch64-unknown-linux-gnu ARM aarch64"
)

for entry in "${targets[@]}"; do
  read -r arch triple elf_sig <<<"$entry"

  # Asset names look like greentic-sorx-dev-v0.2.<run id>-<triple>.tgz; match on
  # the triple suffix rather than rebuilding the prefix, which differs between
  # the dev lane (`greentic-sorx-dev-…`) and a stable release.
  gh release download "$tag" -R "$repo" -D "$work/$arch" \
    -p "*-${triple}.tgz" -p "*-${triple}.tgz.sha256"

  mapfile -t archives < <(find "$work/$arch" -maxdepth 1 -name "*-${triple}.tgz")
  if [ "${#archives[@]}" -ne 1 ]; then
    echo "::error::expected exactly one ${triple} archive in release ${tag}, found ${#archives[@]}" >&2
    exit 1
  fi
  archive="${archives[0]}"
  if [ ! -s "${archive}.sha256" ]; then
    echo "::error::release ${tag} has no .sha256 sidecar for $(basename "$archive")" >&2
    exit 1
  fi

  # The sidecar is `<hex>  <archive name>`; compare the digest explicitly so a
  # sidecar naming a different file cannot pass by checking something else.
  expected="$(awk '{print $1; exit}' "${archive}.sha256")"
  actual="$(sha256sum "$archive" | awk '{print $1}')"
  if [ "$expected" != "$actual" ]; then
    echo "::error::sha256 mismatch for $(basename "$archive"): sidecar ${expected}, file ${actual}" >&2
    exit 1
  fi
  echo "verified $(basename "$archive") sha256=${actual}"

  tar -xzf "$archive" -C "$work/$arch"
  # The archive holds one directory with one executable; its name is
  # `greentic-sorx-dev` on the dev lane and `greentic-sorx` otherwise.
  mapfile -t bins < <(find "$work/$arch" -mindepth 2 -maxdepth 2 -type f -name 'greentic-sorx*' ! -name '*.tgz*')
  if [ "${#bins[@]}" -ne 1 ]; then
    echo "::error::expected exactly one greentic-sorx executable in $(basename "$archive"), found ${#bins[@]}" >&2
    exit 1
  fi

  info="$(file -b "${bins[0]}")"
  case "$info" in
    *ELF*"$elf_sig"*) ;;
    *)
      echo "::error::$(basename "$archive") does not hold a ${arch} ELF binary: ${info}" >&2
      exit 1
      ;;
  esac

  install -D -m 0755 "${bins[0]}" "$out/bin/$arch/greentic-sorx"
  echo "staged $out/bin/$arch/greentic-sorx (${info%%,*})"
done
