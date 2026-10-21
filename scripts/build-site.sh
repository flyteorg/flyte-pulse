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
# Optional size optimization; a missing or failing wasm-opt keeps the module as built.
# Old binaryen releases (such as Ubuntu's 108) write a module whose reference table
# cannot grow; the smoke test below catches that.
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Os "$OUT/pkg/pulse_web_bg.wasm" -o "$OUT/pkg/pulse_web_bg.opt.wasm" \
    && mv "$OUT/pkg/pulse_web_bg.opt.wasm" "$OUT/pkg/pulse_web_bg.wasm" \
    || { echo "wasm-opt failed; keeping the unoptimized module" >&2; rm -f "$OUT/pkg/pulse_web_bg.opt.wasm"; }
fi
cp -R web/. "$OUT/"
cargo run --release -p pulse -- build --state "$STATE" --config-dir config --out "$OUT/data"
if command -v node >/dev/null 2>&1; then
  node scripts/smoke-site.mjs "$OUT"
else
  echo "node not found; skipping the smoke test" >&2
fi
ls -la "$OUT/pkg" "$OUT/data"
