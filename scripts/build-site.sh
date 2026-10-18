#!/usr/bin/env bash
# Build the static site: WebAssembly module, web assets, and the data snapshot.
# Usage: scripts/build-site.sh [STATE] [OUT_DIR]
set -euo pipefail

STATE="${1:-pulse-state.json.gz}"
OUT="${2:-dist}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

cargo build --release -p pulse-web --target wasm32-unknown-unknown
rm -rf "$OUT"
mkdir -p "$OUT/pkg" "$OUT/data"
wasm-bindgen --target web --no-typescript --out-dir "$OUT/pkg" \
  target/wasm32-unknown-unknown/release/pulse_web.wasm
# Optional size optimization; an older or missing wasm-opt keeps the module as built.
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Os "$OUT/pkg/pulse_web_bg.wasm" -o "$OUT/pkg/pulse_web_bg.opt.wasm" \
    && mv "$OUT/pkg/pulse_web_bg.opt.wasm" "$OUT/pkg/pulse_web_bg.wasm" \
    || { echo "wasm-opt failed; keeping the unoptimized module" >&2; rm -f "$OUT/pkg/pulse_web_bg.opt.wasm"; }
fi
cp -R web/. "$OUT/"
cargo run --release -p pulse -- build --state "$STATE" --config-dir config --out "$OUT/data"
ls -la "$OUT/pkg" "$OUT/data"
