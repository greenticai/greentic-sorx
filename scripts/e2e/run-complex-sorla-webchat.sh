#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
sorla_dir="${GREENTIC_SORLA_DIR:-${repo_root}/../greentic-sorla}"
fixture="${repo_root}/tests/e2e/fixtures/complex-property-sorla/sorla.yaml"
work_dir="${GREENTIC_COMPLEX_SORLA_E2E_DIR:-/tmp/sorx-complex-sorla-webchat-e2e}"
pack="${work_dir}/complex-property-sorx-e2e.gtpack"
bundle_dir="${work_dir}/bundle"
log_file="${work_dir}/greentic-sorx-test.log"
sorx_port="${GREENTIC_COMPLEX_SORLA_SORX_PORT:-18877}"
webchat_port="${GREENTIC_COMPLEX_SORLA_WEBCHAT_PORT:-8080}"
webchat_url="http://127.0.0.1:${webchat_port}/v1/web/webchat/demo/"

port_is_open() {
  (echo >"/dev/tcp/127.0.0.1/$1") >/dev/null 2>&1
}

if [ -z "${GREENTIC_COMPLEX_SORLA_SORX_PORT:-}" ]; then
  while port_is_open "${sorx_port}"; do
    sorx_port=$((sorx_port + 1))
  done
fi

if [ ! -d "${sorla_dir}" ]; then
  echo "Skipping complex SoRLa WebChat e2e: greentic-sorla checkout not found at ${sorla_dir}" >&2
  exit 0
fi

rm -rf "${work_dir}"
mkdir -p "${work_dir}"

echo "Building complex SoRLa gtpack from ${fixture}"
cargo run --manifest-path "${sorla_dir}/Cargo.toml" -p greentic-sorla -- \
  pack "${fixture}" \
  --name complex-property-sorx-e2e \
  --version 0.1.0 \
  --out "${pack}"

echo "Starting greentic-sorx test harness on ${webchat_url}"
if command -v setsid >/dev/null 2>&1; then
  setsid bash -c 'cd "$1"; shift; exec "$@"' bash "${repo_root}" \
    cargo run --bin greentic-sorx -- test "${pack}" \
      --force \
      --bundle-dir "${bundle_dir}" \
      --sorx-url "http://127.0.0.1:${sorx_port}" \
      --webchat-url "http://127.0.0.1:${webchat_port}" \
    >"${log_file}" 2>&1 &
else
  (
    cd "${repo_root}"
    cargo run --bin greentic-sorx -- test "${pack}" \
      --force \
      --bundle-dir "${bundle_dir}" \
      --sorx-url "http://127.0.0.1:${sorx_port}" \
      --webchat-url "http://127.0.0.1:${webchat_port}"
  ) >"${log_file}" 2>&1 &
fi
runner_pid="$!"

cleanup() {
  if kill -0 "${runner_pid}" >/dev/null 2>&1; then
    if command -v setsid >/dev/null 2>&1; then
      kill -TERM "-${runner_pid}" >/dev/null 2>&1 || true
    else
      kill "${runner_pid}" >/dev/null 2>&1 || true
    fi
    wait "${runner_pid}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

ready=0
actual_webchat_url=""
for _ in $(seq 1 120); do
  detected_route="$(sed -n 's/^[[:space:]]*Routes:[[:space:]]*//p' "${log_file}" 2>/dev/null | tail -n 1 || true)"
  if [ -n "${detected_route}" ]; then
    actual_webchat_url="${detected_route}"
  fi
  if [ -n "${actual_webchat_url}" ] && curl -fsS "${actual_webchat_url}" >/dev/null 2>&1; then
    ready=1
    break
  fi
  if ! kill -0 "${runner_pid}" >/dev/null 2>&1; then
    echo "greentic-sorx test exited before WebChat became ready" >&2
    cat "${log_file}" >&2
    exit 1
  fi
  sleep 1
done

if [ "${ready}" -ne 1 ]; then
  if [ -z "${actual_webchat_url}" ]; then
    actual_webchat_url="${webchat_url}"
  fi
  echo "WebChat did not become ready: ${actual_webchat_url}" >&2
  cat "${log_file}" >&2
  exit 1
fi

if [ ! -d "${repo_root}/node_modules/@playwright/test" ]; then
  (cd "${repo_root}" && npm install)
fi

echo "Running Playwright complex SoRLa WebChat e2e"
(
  cd "${repo_root}"
  WEBCHAT_E2E_URL="${actual_webchat_url}" npx playwright test tests/e2e/complex-sorla-webchat.spec.js
)
