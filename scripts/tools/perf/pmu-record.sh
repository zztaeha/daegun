#!/bin/sh
set -eu
[ -n "${DEVELOPER_DIR:-}" ] || [ ! -d /Applications/Xcode-beta.app ] \
    || export DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer
xcrun xctrace version >/dev/null 2>&1 || { echo "xctrace not found: install Xcode, or set DEVELOPER_DIR" >&2; exit 1; }
cd "$(dirname "$0")/../../.."

FILTER="$1"
OUT="${2:-${TMPDIR:-/tmp}/attr.$$}"; mkdir -p "$OUT"
MIN_ROWS="${MIN_ROWS:-1200}"
ATTEMPTS="${ATTEMPTS:-8}"

trap "find \"${TMPDIR:-/tmp}\" -maxdepth 1 -name 'instruments*.ktrace' -mmin +60 -exec rm -rf {} + 2>/dev/null || true" EXIT
trap 'exit 130' INT TERM

PROFILE="${PROFILE:-pmu}"
BENCH_TARGET="${BENCH_TARGET:-shaper}"
# Where the target's benches live, from its [[test]] entry in Cargo.toml.
BENCH_PATH=$(python3 -c 'import sys, tomllib
tests = tomllib.load(open("Cargo.toml", "rb")).get("test", [])
print(next((t["path"].rsplit("/", 1)[0] for t in tests if t["name"] == sys.argv[1]), ""))' "$BENCH_TARGET")
[ -n "$BENCH_PATH" ] || { echo "Cargo.toml has no test target named $BENCH_TARGET" >&2; exit 1; }
BIN=$(cargo test --test "$BENCH_TARGET" --profile "$PROFILE" --no-run --message-format=json 2>/dev/null \
      | python3 scripts/tools/perf/pick-test-binary.py "$BENCH_TARGET")
[ -n "$BIN" ] || { echo "no test binary: did the build fail?" >&2; exit 1; }

if [ "$("$BIN" --ignored --list "$FILTER" 2>/dev/null | grep -c ': test$')" -eq 0 ]; then
    echo "$BENCH_TARGET has no #[ignore] benches matching $FILTER, so there is nothing to measure." >&2
    echo "Write them in $BENCH_PATH/, or set BENCH_TARGET." >&2
    exit 1
fi

xcrun xctrace record --template 'CPU Counters' --show-recording-options 2>/dev/null > "$OUT/base.json"
python3 scripts/tools/perf/attr-options.py "$OUT"

ROWS=0
i=0
while [ "$i" -lt "$ATTEMPTS" ]; do
    i=$((i + 1))
    rm -rf "$OUT/t.trace"
    xcrun xctrace record --template 'CPU Counters' --recording-options "$OUT/opts.json" \
        --output "$OUT/t.trace" --launch -- \
        "$BIN" "$FILTER" --ignored --nocapture --test-threads 1 >/dev/null 2>&1 || true
    for t in time-profile kdebug-counters-with-time-sample; do
        xcrun xctrace export --input "$OUT/t.trace" \
            --xpath "/trace-toc/run[@number=\"1\"]/data/table[@schema=\"$t\"]" \
            2>/dev/null > "$OUT/$t.xml" || true
    done
    # Through cat, so a missing file counts 0 too: grep -c prints its 0 and fails, and `|| echo 0` would
    # add a second line that no numeric test can read.
    ROWS=$(cat "$OUT/kdebug-counters-with-time-sample.xml" 2>/dev/null | grep -c '<row>' || true)
    STACKS=$(cat "$OUT/time-profile.xml" 2>/dev/null | grep -c '<row>' || true)
    printf '  attempt %s: %s counter rows, %s stack rows\n' "$i" "$ROWS" "$STACKS" >&2
    if [ "$ROWS" -ge "$MIN_ROWS" ]; then break; fi
done
if [ "$ROWS" -lt "$MIN_ROWS" ]; then
    echo "never reached $MIN_ROWS counter rows in $ATTEMPTS attempts: every one fell back to 1 ms" >&2
    exit 1
fi
echo "$OUT"
