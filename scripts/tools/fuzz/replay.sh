#!/bin/sh
# The inputs that found the engine's fuzzing bugs, kept in regressions/<target>/, through their targets in a plain
# build: any failure, or an input past 10 s, a hang, fails the step. The slow ones' own tests hold their times.
# regressions/c_abi/ seeds c/run.sh instead.
set -e
cd "$(dirname "$0")"
cargo build --release --features threading --bin run >/dev/null 2>&1 || {
  echo "FAILED to build"; cargo build --release --features threading --bin run 2>&1 | grep -E "^error" -A6 | head -30
  exit 1; }
BIN=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
      | tr ',' '\n' | grep -o '"target_directory":"[^"]*"' | cut -d'"' -f4)/release/run
n=0
for dir in regressions/*/; do
  target=$(basename "$dir")
  [ "$target" = c_abi ] && continue
  OUT=$("$BIN" "$target" "$dir"* 2>&1) || { echo "FAILED: $target"; echo "$OUT" | tail -20; exit 1; }
  slow=$(echo "$OUT" | awk '$NF == "s" && $(NF - 1) > 10')
  [ -z "$slow" ] || { echo "FAILED: past 10 s"; echo "$slow"; exit 1; }
  n=$((n + $(echo "$OUT" | grep -c ' s$')))
done
echo "$n inputs, none failed"
