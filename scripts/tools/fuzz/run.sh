#!/bin/sh
set -e
cd "$(dirname "$0")"
COUNT=${1:-500}
[ $# -gt 0 ] && shift

cargo build --release >/dev/null 2>&1 || {
  echo "FAILED to build"; cargo build --release 2>&1 | grep -E "^error" -A6 | head -30; exit 1; }

BIN=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
      | tr ',' '\n' | grep -o '"target_directory":"[^"]*"' | cut -d'"' -f4)/release/daegun-fuzz
exec "$BIN" --count "$COUNT" "$@"
