#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../tests/fixtures/sorx-e2e-guest"
cargo component build --release --target wasm32-wasip2
echo "built: $(pwd)/target/wasm32-wasip2/release/sorx_e2e_guest.wasm"
