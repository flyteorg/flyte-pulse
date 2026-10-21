#!/usr/bin/env bash
# Build the site from the latest published state and serve it at http://127.0.0.1:PORT.
# Usage: scripts/serve-local.sh [PORT]
#   STATE=path   build from a local state file or directory instead of the data branch
set -euo pipefail

PORT="${1:-8000}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ -z "${STATE:-}" ]; then
  STATE=work/state
  git fetch --quiet origin data
  rm -rf "$STATE"
  mkdir -p "$STATE"
  git archive origin/data state | tar -x -C "$STATE" --strip-components=1
fi

scripts/build-site.sh "$STATE" dist
echo "Serving http://127.0.0.1:$PORT (Ctrl-C to stop)"
exec python3 -m http.server "$PORT" --bind 127.0.0.1 --directory dist
