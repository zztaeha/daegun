#!/bin/sh
set -e
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
OUT=${TMPDIR:-/tmp}/daegun-profile
mkdir -p "$OUT"
cd "$ROOT"

BENCH_TARGET="${BENCH_TARGET:-shaper}"
FILTER="${1:-}"
BIN=$(CARGO_PROFILE_RELEASE_DEBUG="line-tables-only" CARGO_PROFILE_RELEASE_STRIP="none" \
  cargo test --test "$BENCH_TARGET" --release --no-run --message-format=json 2>/dev/null \
  | python3 "$ROOT/scripts/tools/perf/pick-test-binary.py" "$BENCH_TARGET")
[ -n "$BIN" ] || { echo "no test binary: is $BENCH_TARGET a [[test]] target in Cargo.toml?" >&2; exit 1; }
cp "$BIN" "$OUT/bench"
if [ "$("$OUT/bench" --ignored --list "$FILTER" 2>/dev/null | grep -c ': test$')" -eq 0 ]; then
    echo "$BENCH_TARGET has no #[ignore] benches${FILTER:+ matching $FILTER}, so there is nothing to measure." >&2
    echo "Set BENCH_TARGET to one that has: shaper, type, machine, api." >&2
    exit 1
fi

nm -n "$OUT/bench" | grep -E " [tT] " > "$OUT/syms.txt"

command -v samply >/dev/null || { echo "samply not found: cargo install samply" >&2; exit 1; }
samply record --save-only -r 20000 -o "$OUT/prof.json.gz" -- \
  "$OUT/bench" --ignored --nocapture --test-threads 1 "$FILTER" >/dev/null 2>&1 \
  || { echo "samply failed to record; on macOS it may need \`samply setup\` first" >&2; exit 1; }

python3 "$ROOT/scripts/tools/profile/report.py" "$OUT/prof.json.gz" "$OUT/syms.txt"
