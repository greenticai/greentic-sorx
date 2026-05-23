#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

python3 - <<'PY'
import filecmp
import json
import pathlib
import re
import subprocess
import sys

try:
    import tomllib
except ModuleNotFoundError:
    print("Python 3.11+ with tomllib is required for release metadata checks.", file=sys.stderr)
    raise

ROOT = pathlib.Path.cwd()
CLI_MANIFEST = ROOT / "crates" / "greentic-sorx-cli" / "Cargo.toml"
PUBLISH_WORKFLOW = ROOT / ".github" / "workflows" / "publish.yml"
BINARIES_WORKFLOW = ROOT / ".github" / "workflows" / "release-binaries.yml"
ROOT_I18N = ROOT / "i18n"
CLI_I18N = ROOT / "crates" / "greentic-sorx-cli" / "i18n"

EXPECTED_TARGETS = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
]


def fail(message: str) -> None:
    print(f"release check failed: {message}", file=sys.stderr)
    sys.exit(1)


metadata = json.loads(
    subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        text=True,
    )
)
packages = {package["name"]: package for package in metadata["packages"]}
cli = packages.get("greentic-sorx")
if cli is None:
    fail("missing greentic-sorx package in cargo metadata")

workspace_version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
if cli["version"] != workspace_version:
    fail(f"greentic-sorx version {cli['version']} does not match workspace version {workspace_version}")

bin_targets = [
    target["name"]
    for target in cli.get("targets", [])
    if "bin" in target.get("kind", [])
]
if bin_targets != ["greentic-sorx"]:
    fail(f"expected exactly one binary target named greentic-sorx, got {bin_targets!r}")

cli_manifest = tomllib.loads(CLI_MANIFEST.read_text())
package = cli_manifest["package"]
include = package.get("include", [])
if "i18n/**" not in include:
    fail("greentic-sorx package.include must contain i18n/** so embedded catalogs are packaged")

binstall = package.get("metadata", {}).get("binstall", {})
expected_binstall = {
    "pkg-url": "{ repo }/releases/download/v{ version }/{ name }-v{ version }-{ target }.{ archive-format }",
    "bin-dir": "{ name }-v{ version }-{ target }",
    "pkg-fmt": "tgz",
    "bin": ["greentic-sorx"],
}
for key, expected in expected_binstall.items():
    actual = binstall.get(key)
    if actual != expected:
        fail(f"package.metadata.binstall.{key} expected {expected!r}, got {actual!r}")
if "overrides" in binstall:
    fail("package.metadata.binstall must not override Windows to zip; release workflow emits tgz for all targets")

publish_text = PUBLISH_WORKFLOW.read_text()
if "branches:" not in publish_text or "      - main" not in publish_text:
    fail("publish workflow must trigger from main branch pushes")
if "Create or verify release tag" not in publish_text:
    fail("publish workflow must create or verify the vX.Y.Z release tag before binary release")
dispatches_binaries_on_ref = any(
    "gh workflow run release-binaries.yml" in line and re.search(r"(^|\s)--ref(\s|=)", line)
    for line in publish_text.splitlines()
)
if not dispatches_binaries_on_ref:
    fail("publish workflow must dispatch release-binaries.yml on the version tag")
if "gh run watch" not in publish_text:
    fail("publish workflow must wait for release-binaries.yml before publishing crates")
if "publish-crates:" not in publish_text or "release-binaries" not in publish_text.split("publish-crates:", 1)[1].split("environment:", 1)[0]:
    fail("publish-crates must depend on release-binaries")
if "Publish must run from the main branch" not in publish_text:
    fail("publish workflow must require a main/master branch ref before releasing")

binaries_text = BINARIES_WORKFLOW.read_text()
if "workflow_dispatch:" not in binaries_text:
    fail("release-binaries workflow must be dispatchable by publish.yml")
if "contents: write" not in binaries_text or "packages: write" not in binaries_text:
    fail("release-binaries workflow must grant contents and packages write permissions to the reusable release workflow")
if "\n  push:" in binaries_text or "\npush:" in binaries_text:
    fail("release-binaries workflow must not trigger directly from pushes; publish.yml orchestrates main releases")
if "if: github.ref_type != 'tag'" not in binaries_text:
    fail("release-binaries branch/manual dispatch job must only re-dispatch from non-tag refs")
if "if: github.ref_type == 'tag'" not in binaries_text:
    fail("release-binaries workflow must run the shared binary release workflow on tag refs")

members = set(metadata.get("workspace_members", []))
publishable = {
    package["name"]: package
    for package in metadata["packages"]
    if (not members or package["id"] in members)
    and package.get("publish") is not False
    and package.get("publish") != []
}
visited = set()
order = []


def visit(name: str) -> None:
    if name in visited:
        return
    visited.add(name)
    package = publishable[name]
    for dependency in package.get("dependencies", []):
        dep_name = dependency["name"]
        if dependency.get("kind") in ("dev", "build"):
            continue
        if dep_name in publishable:
            visit(dep_name)
    order.append(name)


for name in publishable:
    visit(name)

if order != ["greentic-sorx-core", "greentic-sorx-pack", "greentic-sorx"]:
    fail(f"unexpected crates.io publish order: {order!r}")

root_json = sorted(path.name for path in ROOT_I18N.glob("*.json"))
cli_json = sorted(path.name for path in CLI_I18N.glob("*.json"))
if root_json != cli_json:
    fail("crates/greentic-sorx-cli/i18n must contain the same JSON files as root i18n/")
for name in root_json:
    if not filecmp.cmp(ROOT_I18N / name, CLI_I18N / name, shallow=False):
        fail(f"CLI i18n catalog is out of sync: {name}")

print("binstall release assets:")
for target in EXPECTED_TARGETS:
    print(f"  greentic-sorx-v{cli['version']}-{target}.tgz")
print("crates.io publish order:")
for name in order:
    print(f"  {name}")
PY
