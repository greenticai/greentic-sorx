#!/usr/bin/env bash
set -euo pipefail

step() {
  printf '\n==> %s\n' "$1"
}

publishable_crates() {
  cargo metadata --no-deps --format-version 1 | python3 -c '
import json
import sys

metadata = json.load(sys.stdin)
members = set(metadata.get("workspace_members", []))
packages = metadata.get("packages", [])

selected = {
    p["name"]: p
    for p in packages
    if (not members or p["id"] in members)
    and p.get("publish") is not False
    and p.get("publish") != []
}

visited = set()
order = []

def visit(name):
    if name in visited:
        return
    visited.add(name)
    package = selected[name]
    for dep in package.get("dependencies", []):
        dep_name = dep["name"]
        if dep.get("kind") in ("dev", "build"):
            continue
        if dep_name in selected:
            visit(dep_name)
    order.append(name)

for name in selected:
    visit(name)

for name in order:
    package = selected[name]
    publish = package.get("publish")
    if publish is False or publish == []:
        continue
    local_deps = []
    for dep in package.get("dependencies", []):
        if dep.get("kind") in ("dev", "build"):
            continue
        dep_name = dep["name"]
        if dep_name in selected and dep.get("source") is None:
            local_deps.append(dep_name)
    print(name + "\t" + ",".join(local_deps))
'
}

allow_dirty_args() {
  if [ "${CI:-}" = "true" ]; then
    return 0
  fi
  printf '%s\n' "--allow-dirty"
}

step "cargo fmt"
cargo fmt --all -- --check

if [ -x tools/i18n.sh ]; then
  step "i18n"
  tools/i18n.sh validate
  tools/i18n.sh status
fi

step "release metadata"
bash ci/release_check.sh

step "cargo clippy"
cargo clippy --all-targets --all-features -- -D warnings

step "cargo test"
cargo test --all-features

step "cargo build"
cargo build --all-features

step "cargo doc"
cargo doc --no-deps --all-features

crates="$(publishable_crates)"
if [ -z "$crates" ]; then
  step "packaging"
  printf 'No publishable crates detected.\n'
  exit 0
fi

step "packaging"
printf '%s\n' "$crates" | while IFS="$(printf '\t')" read -r crate local_deps; do
  if [ -n "${local_deps:-}" ]; then
    printf '\n-- %s: registry verification deferred\n' "$crate"
    printf 'Skipping cargo package and cargo publish --dry-run because this crate depends on local workspace crate(s): %s.\n' "$local_deps"
    printf 'Cargo requires those dependencies to exist in the registry before preparing the dependent crate for upload.\n'
    printf 'The release workflow publishes dependency crates first, then dry-runs and publishes dependents.\n'
    continue
  fi

  printf '\n-- %s: cargo package --no-verify\n' "$crate"
  cargo package --no-verify -p "$crate" $(allow_dirty_args)

  printf '\n-- %s: cargo package\n' "$crate"
  cargo package -p "$crate" $(allow_dirty_args)

  printf '\n-- %s: cargo publish --dry-run\n' "$crate"
  cargo publish -p "$crate" --dry-run $(allow_dirty_args)
done
