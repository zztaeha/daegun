#!/bin/sh
set -e
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
OUT=${TMPDIR:-/tmp}/daegun-profile
mkdir -p "$OUT"
cd "$ROOT"

# The benches: `lib`, the core's suites in the library's own tests, the shaper's unless FILTER names another
# (tests::daetype::, tests::daekit::), or a [[test]] target such as `api`.
BENCH_TARGET="${BENCH_TARGET:-lib}"
case $BENCH_TARGET in
    lib) TARGET="--lib" ARTIFACT=daegun FILTER="${1:-tests::daeshaper::}" ;;
    *) TARGET="--test=$BENCH_TARGET" ARTIFACT=$BENCH_TARGET FILTER="${1:-}" ;;
esac
BIN=$(CARGO_PROFILE_RELEASE_DEBUG="line-tables-only" CARGO_PROFILE_RELEASE_STRIP="none" \
  cargo test "$TARGET" --release --no-run --message-format=json 2>/dev/null \
  | python3 "$ROOT/scripts/tools/perf/pick-test-binary.py" "$ARTIFACT")
[ -n "$BIN" ] || { echo "no test binary: is $BENCH_TARGET lib or a [[test]] target in Cargo.toml?" >&2; exit 1; }
cp "$BIN" "$OUT/bench"
if [ "$("$OUT/bench" --ignored --list "$FILTER" 2>/dev/null | grep -c ': test$')" -eq 0 ]; then
    echo "$BENCH_TARGET has no #[ignore] benches${FILTER:+ matching $FILTER}, so there is nothing to measure." >&2
    echo "Pick a suite of the library's with FILTER, or set BENCH_TARGET=api." >&2
    exit 1
fi

nm -n "$OUT/bench" | grep -E " [tT] " > "$OUT/syms.txt"

command -v samply >/dev/null || { echo "samply not found: cargo install samply" >&2; exit 1; }
samply record --save-only -r 20000 -o "$OUT/prof.json.gz" -- \
  "$OUT/bench" --ignored --nocapture --test-threads 1 "$FILTER" >/dev/null 2>&1 \
  || { echo "samply failed to record; on macOS it may need \`samply setup\` first" >&2; exit 1; }

python3 "$ROOT/scripts/tools/profile/report.py" "$OUT/prof.json.gz" "$OUT/syms.txt"
