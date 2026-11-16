#!/bin/sh
# abi.c under libFuzzer and ASan, both sides instrumented by one LLVM, from the small test fonts behind fixed choices.
# Usage: run.sh [runs]   (20000 by default; a finding is left beside the build, its path printed)
set -e
cd "$(dirname "$0")/../../../.."
RUNS=${1:-20000}
HOST=$(rustc -vV | awk '/^host:/ {print $2}')
TARGET=$(cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
case "$(uname -s)" in
  Darwin) LINK="" ;;
  *)      LINK="-lgcc_s -lutil -lrt -lpthread -lm -ldl" ;;
esac
LLVM=$(rustc +nightly -vV | awk '/^LLVM version:/ { split($3, v, "."); print v[1] }')
CLANG=""
for c in /opt/homebrew/opt/llvm@"$LLVM"/bin/clang /opt/homebrew/opt/llvm/bin/clang /usr/lib/llvm-"$LLVM"/bin/clang \
         clang-"$LLVM" clang; do
  if "$c" --version 2>/dev/null | head -1 | grep -qE "clang version $LLVM\."; then
    CLANG=$c
    break
  fi
done
if [ -z "$CLANG" ]; then
  echo "no clang on nightly's LLVM $LLVM (brew install llvm@$LLVM, or apt install clang-$LLVM)"; exit 1
fi
OUT=$TARGET/fuzz-c
mkdir -p "$OUT"
COVERAGE="-Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=4 -Cllvm-args=-sanitizer-coverage-pc-table
  -Cllvm-args=-sanitizer-coverage-inline-8bit-counters -Cllvm-args=-sanitizer-coverage-trace-compares"
if ! RUSTFLAGS="-Zsanitizer=address $COVERAGE -Cdebug-assertions -Coverflow-checks" CARGO_TARGET_DIR="$OUT" \
       cargo +nightly rustc --release --features capi --crate-type staticlib --target "$HOST" >"$OUT/build.log" 2>&1 \
   || ! "$CLANG" -std=c11 -g -O1 -fsanitize=fuzzer,address -fprofile-instr-generate -fcoverage-mapping \
       -I src/api/c/include scripts/tools/fuzz/c/abi.c "$OUT/$HOST/release/libdaegun.a" $LINK -o "$OUT/abi" \
       >>"$OUT/build.log" 2>&1; then
  echo "FAILED to build"; tail -10 "$OUT/build.log"; exit 1
fi
rm -rf "$OUT/seeds" "$OUT/corpus"
mkdir -p "$OUT/seeds" "$OUT/corpus"
find assets/test-fonts /System/Library/Fonts/Supplemental -type f -size -96k \
    \( -name '*.ttf' -o -name '*.otf' -o -name '*.ttc' \) 2>/dev/null | sort | python3 -I -c '
import random, sys
out = sys.argv[1]
CHOICES, READERS = 9 * 512, 6 * 512
u32 = lambda b: int.from_bytes(b, "big")
# A variation store is too rare at a random offset for a run to count on: where a font has one, its second seed
# starts the byte readers on it, past the table, the skip, four words and an index.
def store(font):
    if font[:4] == b"ttcf":
        return None
    tables = sorted((font[12 + 16 * i:16 + 16 * i], u32(font[20 + 16 * i:24 + 16 * i])) for i in range(u32(font[4:6])))
    at = [(k, u32(font[start + 4:start + 8])) for k, (tag, start) in enumerate(tables) if tag == b"HVAR"]
    return at[0] if at else None
for n, path in enumerate(line.strip() for line in sys.stdin):
    font = open(path, "rb").read()
    for k in range(2):
        r = random.Random(n * 2 + k)
        choices = bytearray(r.randrange(256) for _ in range(CHOICES))
        if k and (found := store(font)):
            choices[READERS], choices[READERS + 1] = found[0], 0
            choices[READERS + 18], choices[READERS + 20] = 0, 1
            choices[READERS + 21:READERS + 25] = found[1].to_bytes(4, "big")
        open(f"{out}/{n}-{k}", "wb").write(bytes(choices) + font)
' "$OUT/seeds"
cp scripts/tools/fuzz/regressions/c_abi/* "$OUT/seeds/"
SEEDS=$(ls "$OUT/seeds" | wc -l | tr -d ' ')
MAX=$(ls -l "$OUT/seeds" | awk 'NR > 1 && $5 > m { m = $5 } END { print m }')
if ! LLVM_PROFILE_FILE="$OUT/abi.profraw" "$OUT/abi" -seed=1 -runs="$RUNS" -max_len="$MAX" -artifact_prefix="$OUT/" \
     "$OUT/corpus" "$OUT/seeds" >"$OUT/log" 2>&1; then
  echo "FAILED"; grep -E "ERROR|SUMMARY|^ +#[0-9]+ |Test unit written" "$OUT/log" | head -20; exit 1
fi
# A clean run says nothing of code it never reached: every line of abi.c runs, but those that only fail or give up.
TOOLS=$(dirname "$(command -v "$CLANG")")
"$TOOLS/llvm-profdata" merge -sparse "$OUT/abi.profraw" -o "$OUT/abi.profdata"
if ! "$TOOLS/llvm-cov" show "$OUT/abi" -instr-profile="$OUT/abi.profdata" scripts/tools/fuzz/c/abi.c | python3 -I -c '
import re, sys
dead = [m[1].strip() for m in (re.match(r"\s*\d+\|\s*0\|(.*)", line) for line in sys.stdin) if m]
never = [line for line in dead if line.strip("{}\\ ") and not re.search(r"fail\(|^(continue|return);", line)]
if never:
    print("never ran:", *never, sep="\n  ")
sys.exit(bool(never))'; then
  echo "FAILED"; exit 1
fi
echo "$RUNS runs from $SEEDS seeds, both sides under ASan, each line of abi.c reached, unknown options sizes refused"
