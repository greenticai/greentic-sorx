#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

MODE="${1:-all}"
AUTH_MODE="${AUTH_MODE:-auto}"
LOCALE="${LOCALE:-en}"
EN_PATH="${EN_PATH:-i18n/en.json}"
LOCALES_PATH="${LOCALES_PATH:-i18n/locales.json}"
BATCH_SIZE="${I18N_BATCH_SIZE:-200}"
TRANSLATOR_BIN="${TRANSLATOR_BIN:-greentic-i18n-translator}"

usage() {
  cat <<'EOF'
Usage: tools/i18n.sh [translate|validate|status|all]

Environment overrides:
  EN_PATH=...                     English source file path (default: i18n/en.json)
  LOCALES_PATH=...                Locale list file path (default: i18n/locales.json)
  AUTH_MODE=auto|api-key|browser  Translator auth mode for translate (default: auto)
  LOCALE=...                      CLI locale used for translator output (default: en)
  I18N_BATCH_SIZE=<int>           Keys per translation request (default: 200)
  TRANSLATOR_BIN=...              Translator binary name or path (default: greentic-i18n-translator)

Examples:
  tools/i18n.sh all
  AUTH_MODE=api-key tools/i18n.sh translate
  EN_PATH=i18n/en.json tools/i18n.sh validate
EOF
}

log() {
  printf '[i18n] %s\n' "$*"
}

fail() {
  printf '[i18n] error: %s\n' "$*" >&2
  exit 1
}

ensure_translator() {
  if command -v "$TRANSLATOR_BIN" >/dev/null 2>&1; then
    return
  fi

  if command -v greentic-i18n-translator >/dev/null 2>&1; then
    TRANSLATOR_BIN="greentic-i18n-translator"
    return
  fi

  local cargo_bin="${CARGO_HOME:-${HOME:-}/.cargo}/bin/greentic-i18n-translator"
  if [[ -x "$cargo_bin" ]]; then
    TRANSLATOR_BIN="$cargo_bin"
    return
  fi

  command -v cargo-binstall >/dev/null 2>&1 \
    || fail "${TRANSLATOR_BIN} not found and cargo-binstall is unavailable"

  log "installing greentic-i18n-translator via cargo-binstall"
  cargo binstall -y greentic-i18n-translator \
    || fail "failed to install greentic-i18n-translator via cargo-binstall"

  if command -v greentic-i18n-translator >/dev/null 2>&1; then
    TRANSLATOR_BIN="greentic-i18n-translator"
  elif [[ -x "$cargo_bin" ]]; then
    TRANSLATOR_BIN="$cargo_bin"
  else
    fail "greentic-i18n-translator is still unavailable after cargo-binstall"
  fi
}

require_locale_list() {
  [[ -f "$LOCALES_PATH" ]] || fail "missing locale list: ${LOCALES_PATH}"
}

locale_csv() {
  python3 - "$LOCALES_PATH" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as f:
    print(",".join(json.load(f)))
PY
}

translator_supports_batch_size() {
  "$TRANSLATOR_BIN" --help 2>/dev/null | grep -q -- "--batch-size"
}

run_translate() {
  require_locale_list
  local langs
  langs="$(locale_csv)"

  if translator_supports_batch_size; then
    "$TRANSLATOR_BIN" \
      --locale "$LOCALE" \
      translate --langs "$langs" --en "$EN_PATH" --auth-mode "$AUTH_MODE" --batch-size "$BATCH_SIZE"
  else
    "$TRANSLATOR_BIN" \
      --locale "$LOCALE" \
      translate --langs "$langs" --en "$EN_PATH" --auth-mode "$AUTH_MODE"
  fi
  bash tools/sync_cli_i18n.sh
}

run_validate() {
  require_locale_list
  "$TRANSLATOR_BIN" \
    --locale "$LOCALE" \
    validate --langs "$(locale_csv)" --en "$EN_PATH"
}

run_status() {
  require_locale_list
  "$TRANSLATOR_BIN" \
    --locale "$LOCALE" \
    status --langs "$(locale_csv)" --en "$EN_PATH"
}

if [[ "${MODE}" == "-h" || "${MODE}" == "--help" ]]; then
  usage
  exit 0
fi

ensure_translator

case "$MODE" in
  translate) run_translate ;;
  validate) run_validate ;;
  status) run_status ;;
  all)
    run_translate
    run_validate
    run_status
    bash tools/sync_cli_i18n.sh
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    usage
    exit 2
    ;;
esac
