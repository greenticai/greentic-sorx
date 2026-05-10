#!/usr/bin/env bash
set -euo pipefail

provider="memory"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --provider)
      provider="${2:-}"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

case "$provider" in
  memory)
    cargo test -p greentic-sorx --test landlord_tenant_e2e landlord_tenant_memory_e2e_runs_from_gtpack -- --nocapture
    ;;
  foundationdb)
    echo "FoundationDB e2e is not automated yet; SORX currently exposes an unavailable adapter boundary." >&2
    cargo test -p greentic-sorx --test landlord_tenant_e2e foundationdb_answers_are_present_for_manual_e2e -- --nocapture
    ;;
  *)
    echo "unsupported provider: $provider" >&2
    exit 2
    ;;
esac
