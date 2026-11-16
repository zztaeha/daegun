#!/bin/sh
# Nothing here is skipped: a missing compiler, toolchain or target fails the gate with the command that installs it.
set -e
cd "$(dirname "$0")/../../.."

# Where cargo builds, which CARGO_TARGET_DIR or a config can move away from ./target.
TARGET=$(cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
HOST=$(rustc -vV | awk '/^host:/ {print $2}')
# The system libraries a static library needs beside it, as the README's table names them for a platform.
links() { sed -nE "s/^\| $1 \| \`[^\`]*daegun\.(a|lib) ?([^\`]*)\` \|$/\2/p" README.md; }
# What Rust's standard library asks for on a target, less what the compiler driver adds itself (libc, libSystem with
# libm, cl's /MD runtime, on musl gcc's unwinder for Rust's LLVM one). Its arguments: the target and a scratch folder.
std_links() {
  case $1 in
    *-apple-*) DRIVER=" -lc -lSystem -lm " ;;
    *-musl) DRIVER=" -lc -lunwind " ;;
    *) DRIVER=" -lc /defaultlib:msvcrt " ;;
  esac
  printf 'pub fn f() {}\n' >"$2/probe.rs"
  rustc --edition 2024 --crate-type staticlib --print native-static-libs --target "$1" -o "$2/probe.a" "$2/probe.rs" \
    2>&1 | sed -n 's/.*native-static-libs: //p' | tr ' ' '\n' \
    | awk -v driver="$DRIVER" 'NF && index(driver, " " $0 " ") == 0' | paste -sd' ' -
}
case "$(uname -s)" in
  Darwin) ROW='macOS, iOS' ;;
  *)      ROW=Linux ;;
esac
LINK=$(links "$ROW")

printf 'release build .... '
cargo build --release >/dev/null 2>&1 \
  || { echo "FAILED"; cargo build --release 2>&1 | grep -E "^error" -A6 | head -30; exit 1; }
echo ok

printf 'debug build ...... '
cargo build >/dev/null 2>&1 || { echo "FAILED"; cargo build 2>&1 | grep -E "^error" -A6 | head -30; exit 1; }
echo ok

printf 'tests ............ '
OUT=$(cargo test --tests 2>&1) || { echo "FAILED"; echo "$OUT" | grep -E "^(error|failures:)" -A8 | head -40; exit 1; }
N=$(echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 }')
[ "$N" -eq 0 ] && echo "0 (the suite is empty)" || echo "$N passed"

# The header's DAEGUN_ABI_VERSION, 0x020000u, as 2.0.0.
header_abi() {
  sed -nE 's/^#define DAEGUN_ABI_VERSION 0x([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})u$/\1 \2 \3/p' \
    src/api/c/include/daegun/base.h | { read -r major minor patch; echo "$((0x$major)).$((0x$minor)).$((0x$patch))"; }
}

# Every feature set a caller can build, as a lint or an item can live behind any one of them.
FEATURE_SETS='default
--no-default-features
--features threading
--features capi
--features capi,dictionaries
--no-default-features --features dictionaries
--no-default-features --features capi'
each_feature_set() {
  echo "$FEATURE_SETS" | while read -r f; do
    [ "$f" = default ] && f=""
    OUT=$("$@" $f 2>&1) || true
    [ "$(echo "$OUT" | grep -cE '^(warning|error)')" -eq 0 ] \
      || { echo "issues with '${f:-default}'"; echo "$OUT" | grep -E '^(warning|error)' -A6 | head -20; return 1; }
  done
}

printf 'clippy ........... '
each_feature_set cargo clippy --all-targets || exit 1
echo clean

printf 'rustdoc links .... '
for f in "" --all-features; do
  if [ "$(cargo doc -p daegun --no-deps $f 2>&1 | grep -cE '^(warning|error)')" -ne 0 ]; then
    echo "issues"; cargo doc -p daegun --no-deps $f 2>&1 | grep -E '^(warning|error)' -A4 | head -10; exit 1
  fi
done
echo clean

printf 'fuzz ............. '
# Beside the repo's fonts, macOS's, whose kerx, ankr and trak tables no other seed holds.
if OUT=$(sh scripts/tools/fuzz/run.sh 2000 --extra /System/Library/Fonts/Supplemental 2>&1); then
  echo "$OUT"
else
  echo "FAILED"; echo "$OUT" | head -20; exit 1
fi

printf 'fuzz regressions . '
if OUT=$(sh scripts/tools/fuzz/replay.sh 2>&1); then
  echo "$OUT"
else
  echo "$OUT" | head -20; exit 1
fi

printf 'fuzz reach ....... '
python3 -I scripts/tools/fuzz/fuzz-reach.py || exit 1

printf 'fuzz the c abi ... '
if OUT=$(sh scripts/tools/fuzz/c/run.sh 2>&1); then
  echo "$OUT"
else
  echo "$OUT" | head -20; exit 1
fi

printf 'feature threading '
[ "$(cargo build --features threading 2>&1 | grep -cE '^(warning|error)')" -eq 0 ] && echo clean \
  || { echo "issues"; exit 1; }

printf 'threading tests .. '
OUT=$(cargo test --features threading --tests 2>&1) || {
  echo "FAILED"; echo "$OUT" | grep -E "^(error|failures:)" -A8 | head -40; exit 1; }
TN=$(echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 }')
echo "$TN passed"

# Without std, whose thread spares and the like some tests promise only under a cfg that must say so.
printf 'no-std tests ..... '
OUT=$(cargo test --no-default-features --tests 2>&1) || {
  echo "FAILED"; echo "$OUT" | grep -E "^(error|failures:)" -A8 | head -40; exit 1; }
NN=$(echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 }')
echo "$NN passed"

# With the word lists too, which nothing else here builds.
printf 'capi tests ....... '
OUT=$(cargo test --features capi,dictionaries --tests 2>&1) || {
  echo "FAILED"; echo "$OUT" | grep -E "^(error|failures:)" -A8 | head -40; exit 1; }
CN=$(echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 }')
echo "$CN passed"

printf 'no-default ....... '
[ "$(cargo build --no-default-features 2>&1 | grep -cE '^(warning|error)')" -eq 0 ] && echo clean \
  || { echo "issues"; exit 1; }

# A microcontroller with atomic compare-and-swap builds the crate without std, and one without is told why it can't.
printf 'no_std targets ... '
for t in thumbv7em-none-eabi thumbv6m-none-eabi; do
  rustup target list --installed | grep -q "^$t\$" || { echo "no $t target (rustup target add $t)"; exit 1; }
done
for f in "" "--features dictionaries"; do
  if [ "$(cargo build --no-default-features $f --target thumbv7em-none-eabi 2>&1 | grep -cE '^(warning|error)')" -ne 0 ]
  then
    echo "the crate${f:+ with $f} does not build clean for thumbv7em-none-eabi"; exit 1
  fi
done
OUT=$(cargo build --no-default-features --target thumbv6m-none-eabi 2>&1 || true)
if ! echo "$OUT" | grep -q 'needs atomic compare-and-swap at pointer width'; then
  echo "thumbv6m-none-eabi, which lacks atomic compare-and-swap, is not told so"; exit 1
fi
rustup target list --installed | grep -q '^wasm32-unknown-unknown$' \
  || { echo "no wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown)"; exit 1; }
for f in "" "--no-default-features"; do
  if [ "$(cargo build $f --target wasm32-unknown-unknown 2>&1 | grep -cE '^(warning|error)')" -ne 0 ]; then
    echo "the crate${f:+ with $f} does not build clean for wasm32-unknown-unknown"; exit 1
  fi
done
echo "thumbv7em-none-eabi builds clean, with the word lists too, thumbv6m-none-eabi is told it lacks the atomics, and" \
  "wasm32-unknown-unknown builds clean with its default features and without"

printf 'c abi ............ '
if ! command -v cc >/dev/null 2>&1 || ! command -v c++ >/dev/null 2>&1; then
  echo "no C or C++ compiler"; exit 1
elif ! cargo rustc --features capi --crate-type staticlib >/dev/null 2>&1; then
  echo "the C ABI does not build"; cargo rustc --features capi --crate-type staticlib 2>&1 | tail -20; exit 1
else
  CI=src/api/c/include
  CW=src/api/c/tests
  if ! cargo test --features capi --lib --quiet >/dev/null 2>&1; then
    echo "the C ABI's own tests failed"; cargo test --features capi --lib 2>&1 | tail -20; exit 1
  fi
  CT=$(mktemp -d)
  if [ "$(std_links "$HOST" "$CT")" != "$LINK" ]; then
    echo "the README's $ROW line links '$LINK', but Rust's standard library asks for '$(std_links "$HOST" "$CT")'"
    rm -rf "$CT"; exit 1
  fi
  # base.h shows the README's table to a reader of the header: the two hold the same lines.
  for pair in 'macOS, iOS|macOS / iOS' 'Linux|Linux' 'Linux, musl|Linux, musl' 'Android|Android' 'Windows|Windows' \
              'MinGW|MinGW'; do
    readme=$(sed -nE "s/^\| ${pair%%|*} \| \`([^\`]*)\` \|$/\1/p" README.md)
    header=$(sed -nE "s#^ \*     ${pair#*|} +(.+)\$#\1#p" "$CI/daegun/base.h")
    if [ -z "$readme" ] || [ "$readme" != "$header" ]; then
      echo "base.h's ${pair#*|} line reads '$header', the README's '$readme'"; rm -rf "$CT"; exit 1
    fi
  done
  if ! cc -std=c11 -Wall -Wextra -Werror -I "$CI" "$CW/roundtrip.c" \
       "$TARGET/debug/libdaegun.a" $LINK -o "$CT/rt" 2>"$CT/cc.log" \
     || ! cc -std=c11 -Wall -Wextra -Werror -I "$CI" "$CW/latency.c" \
       "$TARGET/debug/libdaegun.a" $LINK -o "$CT/latency" 2>>"$CT/cc.log" \
     || ! c++ -std=c++17 -Wall -Wextra -Werror -pedantic -I "$CI" "$CW/handles.cpp" \
       "$TARGET/debug/libdaegun.a" $LINK -o "$CT/handles" 2>>"$CT/cc.log" \
     || ! cc -std=c11 -Wall -Wextra -Werror -pedantic -I "$CI" "$CW/readme.c" \
       "$TARGET/debug/libdaegun.a" $LINK -o "$CT/readme" 2>>"$CT/cc.log"; then
    echo "a header or test does not compile"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
  # A shared library of yours linking the archive as the README says exports its own calls alone.
  printf '%s\n' '#include "daegun.h"' 'int plugin_paths(void) {' '    daegun_path *p = daegun_path_new();' \
    '    daegun_path_free(p);' '    return p != NULL;' '}' >"$CT/plugin.c"
  HIDDEN=$(sed -nE 's/.*`(-Wl,-hidden-l[^`]*)`.*/\1/p' README.md)
  if [ "$(uname -s)" = Darwin ] && { [ -z "$HIDDEN" ] \
       || ! cc -dynamiclib -std=c11 -I "$CI" "$CT/plugin.c" -L "$TARGET/debug" $HIDDEN -o "$CT/libplugin.dylib" \
         2>>"$CT/cc.log" || nm -gU "$CT/libplugin.dylib" | grep -v ' _plugin_paths$' | grep -q .; }; then
    echo "a dylib linking daegun with the README's '$HIDDEN' exports daegun's symbols"; cat "$CT/cc.log"
    rm -rf "$CT"; exit 1
  fi
  if ! OUT=$("$CT/rt" assets/test-fonts/inter/InterVariable.ttf 2>&1); then
    echo "round trip failed"; echo "$OUT"; rm -rf "$CT"; exit 1
  fi
  # The README's C blocks, which readme.rs holds to readme.c line for line.
  if ! OUT=$("$CT/readme" assets/test-fonts/inter/InterVariable.ttf 2>&1); then
    echo "the README's C failed"; echo "$OUT"; rm -rf "$CT"; exit 1
  fi
  if ! OUT=$("$CT/handles" 2>&1); then
    echo "the C++ handles failed"; echo "$OUT"; rm -rf "$CT"; exit 1
  fi
  # Every count the headers give clang's bounds safety, read by a program built with it; one past a view traps.
  if ! clang -std=c11 -fbounds-safety -Wall -Wextra -Werror -I "$CI" "$CW/bounds.c" "$TARGET/debug/libdaegun.a" $LINK \
       -o "$CT/bounds" 2>"$CT/cc.log"; then
    echo "the views do not build under clang's bounds safety"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
  elif ! OUT=$("$CT/bounds" 2>&1); then
    echo "the bounds program failed"; echo "$OUT"; rm -rf "$CT"; exit 1
  fi
  # Run by a shell of its own, whose notice of the trap goes where its stderr does.
  got=$(sh -c '"$1" past; echo $?' sh "$CT/bounds" 2>/dev/null)
  if [ "$got" -le 128 ]; then
    echo "clang's bounds safety reads no count on a returned view: one past it ended with $got"; rm -rf "$CT"; exit 1
  fi
  # Rule 2 for every export: NULL for each pointer must answer DAEGUN_NULL, not crash.
  python3 -I scripts/tools/c-api/null-calls.py > "$CT/null_calls.c"
  if ! cc -std=c11 -Wall -Wextra -Werror -I "$CI" "$CT/null_calls.c" "$TARGET/debug/libdaegun.a" $LINK \
       -o "$CT/nc" 2>"$CT/cc.log" || ! OUT=$("$CT/nc" 2>&1); then
    echo "a call broke rule 2"; cat "$CT/cc.log"; echo "$OUT"; rm -rf "$CT"; exit 1
  fi
  # UBSan reports and carries on unless told not to, which would leave the exit status clean. Leaks are the
  # instrumented step's, below: this cc's ASan may have no leak checker.
  SAN="-g -fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer"
  if ! cc -std=c11 $SAN -I "$CI" "$CW/roundtrip.c" "$TARGET/debug/libdaegun.a" $LINK -o "$CT/rt-san" \
       2>"$CT/cc.log" \
     || ! c++ -std=c++17 $SAN -I "$CI" "$CW/handles.cpp" "$TARGET/debug/libdaegun.a" $LINK -o "$CT/handles-san" \
       2>>"$CT/cc.log" \
     || ! cc -std=c11 $SAN -I "$CI" "$CW/readme.c" "$TARGET/debug/libdaegun.a" $LINK -o "$CT/readme-san" \
       2>>"$CT/cc.log"; then
    echo "the tests do not build under asan+ubsan"; head -10 "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
  if ! OUT=$("$CT/rt-san" assets/test-fonts/inter/InterVariable.ttf 2>&1) || ! OUT=$("$CT/handles-san" 2>&1) \
     || ! OUT=$("$CT/readme-san" assets/test-fonts/inter/InterVariable.ttf 2>&1); then
    echo "failed under asan+ubsan"
    echo "$OUT" | grep -E "FAIL|failed|ERROR|SUMMARY|runtime error|^ +#[0-9]+ " | head -30; rm -rf "$CT"; exit 1
  fi
  # Once more with the word lists, built apart so the plain library stays the one the later steps link.
  if ! CARGO_TARGET_DIR="$TARGET/dictionaries" cargo rustc --features capi,dictionaries --crate-type staticlib \
       >/dev/null 2>&1 \
     || ! cc -std=c11 -Wall -Wextra -Werror -I "$CI" "$CW/roundtrip.c" \
       "$TARGET/dictionaries/debug/libdaegun.a" $LINK -o "$CT/rt-dict" 2>"$CT/cc.log" \
     || ! OUT=$("$CT/rt-dict" assets/test-fonts/inter/InterVariable.ttf 2>&1); then
    echo "round trip failed with the word lists"; cat "$CT/cc.log"; echo "$OUT"; rm -rf "$CT"; exit 1
  fi
  echo "round trip, the README's C, C++ handles and NULL calls ok, clean under asan+ubsan, and with the word" \
    "lists; the README's $ROW line links what Rust's standard library asks for, and base.h's table is the README's"
  rm -rf "$CT"
fi

# Each header alone, and all of them in every C and C++ dialect a caller might build with, under clang and GCC
# both: they disagree on what -pedantic flags. GCC is found by what it says it is, as macOS names clang gcc.
printf 'c headers ........ '
GCC=""
GXX=""
for v in 16 15 14 ""; do
  c=gcc${v:+-$v}
  x=g++${v:+-$v}
  if "$c" --version 2>/dev/null | grep -q "Free Software Foundation" \
     && "$x" --version 2>/dev/null | grep -q "Free Software Foundation"; then
    GCC=$c
    GXX=$x
    break
  fi
done
if [ -z "$GCC" ]; then
  echo "no GCC (brew install gcc, or apt install gcc g++)"; exit 1
fi
CI=src/api/c/include
CT=$(mktemp -d)
for h in "$CI"/daegun/*.h; do
  printf '#include <daegun/%s>\nint main(void) { return 0; }\n' "$(basename "$h")" >"$CT/one.c"
  if ! cc -std=c11 -Wall -Wextra -Werror -pedantic -I "$CI" -fsyntax-only "$CT/one.c" 2>"$CT/cc.log"; then
    echo "$(basename "$h") does not compile alone"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
done
printf '%s\n' '#include "daegun.h"' 'int main(void) {' \
  "    const char s[4] = { DAEGUN_UNTAG(DAEGUN_TAG('w', 'g', 'h', 't')) };" '    return s[0] != 0x77;' '}' >"$CT/all.c"
cp "$CT/all.c" "$CT/all.cpp"
for std in c11 c17 c23 c2y; do
  for c in cc "$GCC"; do
    if ! "$c" -std=$std -Wall -Wextra -Werror -pedantic -I "$CI" -fsyntax-only "$CT/all.c" 2>"$CT/cc.log"; then
      echo "the headers fail as $std under $c"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
    fi
  done
done
for std in c++11 c++14 c++17 c++20 c++23 c++26; do
  for x in c++ "$GXX"; do
    if ! "$x" -std=$std -Wall -Wextra -Werror -pedantic -Wold-style-cast -I "$CI" -fsyntax-only "$CT/all.cpp" \
         2>"$CT/cc.log"; then
      echo "the headers fail as $std under $x"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
    fi
  done
done
# 32-bit x86 aligns a double to 4, so the layout asserts take their other branch there.
if ! command -v clang >/dev/null 2>&1; then
  echo "no clang (xcode-select --install, or apt install clang)"; rm -rf "$CT"; exit 1
elif ! clang -std=c11 --target=i386-linux-gnu -Wall -Wextra -Werror -pedantic -I "$CI" -fsyntax-only "$CT/all.c" \
     2>"$CT/cc.log"; then
  echo "the headers fail on i386"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
fi
# Each doc comment parses as one, in C and C++, so clang hands it on whole.
for x in "clang -std=c11 all.c" "clang++ -std=c++17 all.cpp"; do
  set -- $x
  if ! "$1" "$2" -Wdocumentation -Wdocumentation-unknown-command -Werror -I "$CI" -fsyntax-only "$CT/$3" \
       2>"$CT/cc.log"; then
    echo "$1 reads a doc comment wrong"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
done
# A status dropped warns wherever base.h says it does, and GCC ties each allocator to its free.
printf '%s\n' '#include "daegun.h"' 'int main(void) {' '    daegun_font *f = 0;' '    daegun_font_open(0, 0, &f);' \
  '    return 0;' '}' >"$CT/drop.c"
cp "$CT/drop.c" "$CT/drop.cpp"
printf '%s\n' '#include "daegun.h"' 'int main(void) {' '    daegun_path *p = daegun_path_new();' \
  '    daegun_table_map_free((daegun_table_map *)(void *)p);' '    return 0;' '}' >"$CT/dealloc.c"
for run in "cc -std=c11 drop.c unused-result" "cc -std=c23 drop.c unused-result" "$GCC -std=c23 drop.c unused-result" \
    "c++ -std=c++11 drop.cpp unused-result" "$GXX -std=c++17 drop.cpp unused-result" \
    "$GCC -std=c11 dealloc.c mismatched-dealloc"; do
  set -- $run
  if "$1" "$2" -Wall -Wextra -Werror -I "$CI" -c -o /dev/null "$CT/$3" >"$CT/cc.log" 2>&1 \
     || ! grep -q "Werror.*$4" "$CT/cc.log"; then
    echo "$1 $2 does not warn on $3"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
done
# clang reads which pointers may be NULL: one to each required pointer warns, and one to a nullable doesn't.
printf '%s\n' '#include "daegun.h"' 'int main(void) {' '    daegun_font_free(0);' \
  '    return daegun_font_glyph_names(0, 0) != DAEGUN_NULL;' '}' >"$CT/null.c"
cp "$CT/null.c" "$CT/null.cpp"
for run in "clang -std=c11 null.c" "clang++ -std=c++17 null.cpp"; do
  set -- $run
  "$1" "$2" -Wall -Wextra -I "$CI" -c -o /dev/null "$CT/$3" >"$CT/cc.log" 2>&1
  if [ "$(grep -c 'Wnonnull' "$CT/cc.log")" != 2 ] || grep -q "$3:3:" "$CT/cc.log"; then
    echo "$1 does not read the nullability marks"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
done
# The C tests build clean under GCC too, whose bounds checks read the header's array lengths and allocator sizes.
for run in "$GCC -std=c11 roundtrip.c" "$GCC -std=c11 latency.c" "$GXX -std=c++17 handles.cpp"; do
  set -- $run
  if ! "$1" "$2" -O2 -Wall -Wextra -Werror -pedantic -I "$CI" -c -o /dev/null "src/api/c/tests/$3" \
       >"$CT/cc.log" 2>&1; then
    echo "$3 fails under $1"; head -30 "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
done
# GCC's analyzer takes the three allocators that never return NULL at their word, so a new path passed where NULL
# isn't allowed draws no warning.
printf '%s\n' '#include "daegun.h"' \
  '__attribute__((nonnull)) static int drawn(daegun_path *p) { return daegun_path_move_to(p, 0, 0); }' \
  'int main(void) {' '    daegun_path *p = daegun_path_new();' '    int st = drawn(p);' '    daegun_path_free(p);' \
  '    return st;' '}' >"$CT/nonnull.c"
if ! "$GCC" -std=c11 -fanalyzer -Wall -Wextra -Werror -I "$CI" -c -o /dev/null "$CT/nonnull.c" >"$CT/cc.log" 2>&1; then
  echo "GCC's analyzer takes daegun_path_new as possibly NULL"; head -10 "$CT/cc.log"; rm -rf "$CT"; exit 1
fi
# A short array and NULL with a length warn, in C and C++; an exact array, NULL with none, a buffer the call takes
# NULL for and DAEGUN_ABI_COMPATIBLE don't.
printf '%s\n' '#include "daegun.h"' 'int bounds(const daegun_font *f) {' '    daegun_tag tags[2];' '    size_t n;' \
  '    char text[4] = "abc";' '    daegun_shaped_run *run = 0;' \
  '    return daegun_font_table_tags(f, tags, 4, &n) + daegun_font_shape(f, text, 8, 0, 0, &run)' \
  '        + daegun_font_shape(f, 0, 2, 0, 0, &run);' '}' \
  'int fine(const daegun_font *f) {' '    daegun_tag tags[4];' '    uint32_t gids[4];' '    size_t n;' \
  '    return daegun_font_table_tags(f, tags, 4, &n) + daegun_font_table_tags(f, 0, 0, &n)' \
  '        + daegun_font_coverage(f, 0, gids, 4, &n) + (int)DAEGUN_ABI_COMPATIBLE(daegun_abi_version());' '}' \
  >"$CT/bounds.c"
for run in "$GCC c" "$GXX c++"; do
  set -- $run
  "$1" -x "$2" -O2 -Wall -Wextra -pedantic -I "$CI" -c -o /dev/null "$CT/bounds.c" >"$CT/cc.log" 2>&1
  if [ "$(grep -cE 'Wstringop-overflow|Wstringop-overread|Wnonnull' "$CT/cc.log")" != 3 ] \
     || grep -q "In function .*fine" "$CT/cc.log"; then
    echo "$1 does not read the array lengths as it should"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
done
# A view's pointer carries its count to GCC 16 in C: an index within it runs, and one past it traps.
if [ "$("$GCC" -dumpversion | cut -d. -f1)" -lt 16 ]; then
  echo "$GCC is older than 16, which reads a view's count (brew install gcc, or apt install gcc-16)"; rm -rf "$CT"
  exit 1
fi
printf '%s\n' '#include "daegun.h"' 'int main(int argc, char **argv) {' \
  '    static const uint8_t bytes[4] = {1, 2, 3, 4};' '    daegun_bytes view = {bytes, 2};' '    (void)argv;' \
  '    return view.data[argc + PAST] == 2 ? 0 : 1;' '}' >"$CT/view.c"
for past in 0 1; do
  "$GCC" -std=c11 -O2 -fsanitize=bounds -fsanitize-trap=bounds -DPAST=$past -I "$CI" "$CT/view.c" -o "$CT/view" \
    >"$CT/cc.log" 2>&1 || { echo "the view probe does not build"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1; }
  # Run by a shell of its own, whose notice of the trap goes where its stderr does.
  got=$(sh -c '"$1"; echo $?' sh "$CT/view" 2>/dev/null)
  if { [ $past = 0 ] && [ $got != 0 ]; } || { [ $past = 1 ] && [ $got -le 128 ]; }; then
    echo "$GCC reads no count on a view: index $((1 + past)) of 2 ended with $got"; rm -rf "$CT"; exit 1
  fi
done
# Array parameters carry their lengths to -fbounds-safety: a short array is an error, an exact one or a NULL a call
# allows builds, and the headers build alone with the attributes only, as Swift's C interop reads them.
for n in 2 4; do
  printf '%s\n' '#include "daegun.h"' 'int f(const daegun_font *font) {' "    uint32_t gids[$n] = {0};" \
    '    double out[4];' '    size_t count = 0;' '    (void)daegun_font_coverage(font, NULL, gids, 4, &count);' \
    '    return daegun_font_advances(font, gids, 4, 0, 0, out);' '}' >"$CT/safe$n.c"
done
if ! clang -std=c11 -fbounds-safety -Wall -Wextra -Werror -I "$CI" -fsyntax-only "$CT/safe4.c" >"$CT/cc.log" 2>&1 \
   || ! clang -std=c11 -fexperimental-bounds-safety-attributes -Wall -Wextra -pedantic -Werror -I "$CI" \
        -fsyntax-only "$CT/all.c" >>"$CT/cc.log" 2>&1; then
  echo "the headers do not build under clang's bounds safety (Xcode's clang has it)"; cat "$CT/cc.log"
  rm -rf "$CT"; exit 1
elif clang -std=c11 -fbounds-safety -I "$CI" -fsyntax-only "$CT/safe2.c" >"$CT/cc.log" 2>&1 \
     || [ "$(grep -c "which has 2 elements" "$CT/cc.log")" -ne 2 ]; then
  echo "clang's bounds safety reads no array length"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
fi
# A #pragma pack in force at the include moves no field.
printf '%s\n' '#pragma pack(1)' '#include "daegun.h"' 'typedef struct { char c; int i; } after;' \
  '_Static_assert(sizeof(daegun_coord) == 16, "packed");' '_Static_assert(sizeof(after) == 5, "not restored");' \
  >"$CT/pack.c"
for cc in clang "$GCC"; do
  if ! "$cc" -std=c11 -Wall -Wextra -Werror -I "$CI" -fsyntax-only "$CT/pack.c" >"$CT/cc.log" 2>&1; then
    echo "$cc: a #pragma pack before the include moves the layouts"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
done
rm -rf "$CT"
echo "each alone, and together as C11 to C2y and C++11 to C++26, under clang and $GCC, and on i386; a dropped" \
  "status and a mismatched free warn, and NULL where a pointer is required; the C tests build clean under $GCC," \
  "which warns of a short array and NULL with a length and of nothing a call allows, and traps an index past a" \
  "view's count; clang's bounds safety refuses a short array"

# The shared library, built as the README builds it, is named by the ABI's major: its install name on macOS, its
# SONAME on Linux, linked there by zig from a Mac.
printf 'shared library ... '
MAJOR=$(header_abi | cut -d. -f1)
# The macOS line goes on past its backslashes.
MAC=$(python3 -I -c 'import re; t = re.sub(r"\\\n\s*", "", open("README.md").read())
m = re.search(r"^cargo rustc (.*install_name,@rpath/libdaegun\.[0-9]+\.dylib .*)$", t, re.M); print(m and m.group(1))')
LNX=$(sed -nE 's/^cargo rustc (.*-soname,libdaegun\.so\.[0-9]+)$/\1/p' README.md)
READELF=$(command -v llvm-readelf || command -v readelf || echo /opt/homebrew/opt/llvm/bin/llvm-readelf)
case "$MAC|$LNX" in
  *"libdaegun.$MAJOR.dylib "*"|"*"libdaegun.so.$MAJOR") ;;
  *) echo "the README's shared builds don't name ABI major $MAJOR: '$MAC', '$LNX'"; exit 1 ;;
esac
if [ "$(uname -s)" = Darwin ]; then
  if ! command -v cargo-zigbuild >/dev/null 2>&1 || ! command -v zig >/dev/null 2>&1; then
    echo "no cargo-zigbuild (cargo install --locked cargo-zigbuild, brew install zig)"; exit 1
  fi
  CARGO_TARGET_DIR="$TARGET/shared" cargo rustc -q $MAC >/dev/null 2>&1 || { echo "the macOS build failed"; exit 1; }
  # Against glibc 2.17, as the release builds it.
  CARGO_TARGET_DIR="$TARGET/shared" cargo-zigbuild rustc -q ${LNX%% -- *} --target x86_64-unknown-linux-gnu.2.17 \
    -- ${LNX#* -- } >/dev/null 2>&1 || { echo "the Linux build failed"; exit 1; }
  NAME=$(otool -D "$TARGET/shared/release/libdaegun.dylib" | tail -1)
  SO="$TARGET/shared/x86_64-unknown-linux-gnu/release/libdaegun.so"
else
  CARGO_TARGET_DIR="$TARGET/shared" cargo rustc -q $LNX >/dev/null 2>&1 || { echo "the Linux build failed"; exit 1; }
  NAME=@rpath/libdaegun.$MAJOR.dylib
  SO="$TARGET/shared/release/libdaegun.so"
fi
SONAME=$("$READELF" -d "$SO" | sed -nE 's/.*SONAME.*\[(.*)\]/\1/p')
if [ "$NAME" != "@rpath/libdaegun.$MAJOR.dylib" ] || [ "$SONAME" != "libdaegun.so.$MAJOR" ]; then
  echo "the shared library is named '$NAME' and '$SONAME', not by ABI major $MAJOR"; exit 1
fi
# The dylib versioned as cargo-c's, and the .so needing no glibc past 2.17, Rust's floor.
ABI=$(header_abi)
if [ "$(uname -s)" = Darwin ] \
   && ! otool -L "$TARGET/shared/release/libdaegun.dylib" | sed -n 2p \
        | grep -q "(compatibility version $MAJOR.0.0, current version $ABI)"; then
  echo "the dylib is not versioned $ABI, compatible from $MAJOR.0.0"; otool -L "$TARGET/shared/release/libdaegun.dylib"
  exit 1
fi
# An app's bundler rewrites the install name, which takes the room the README's line leaves in the header.
if [ "$(uname -s)" = Darwin ]; then
  cp "$TARGET/shared/release/libdaegun.dylib" "$TARGET/shared/renamed.dylib"
  if ! install_name_tool -id "@rpath/$(printf '%0200d' 0 | tr 0 x)/libdaegun.$MAJOR.dylib" \
       "$TARGET/shared/renamed.dylib" 2>"$TARGET/shared/renamed.log"; then
    echo "the dylib has no room for a longer install name"; cat "$TARGET/shared/renamed.log"; exit 1
  fi
fi
GLIBC=$("$READELF" -V "$SO" | grep -oE 'GLIBC_[0-9]+\.[0-9]+' | sort -t. -k2 -n | tail -1)
case "$GLIBC" in
  GLIBC_2.[0-9]|GLIBC_2.1[0-7]) ;;
  *) echo "the .so needs $GLIBC, past glibc 2.17"; exit 1 ;;
esac
SHARED="$SO"
[ "$(uname -s)" = Darwin ] && SHARED="$TARGET/shared/release/libdaegun.dylib $SO"
# Each exports the header's calls and nothing else, which another library could collide with.
abi_out=$(python3 -I scripts/tools/c-api/abi-parity.py $SHARED) || { echo "issues"; echo "$abi_out"; exit 1; }
# A program finds the library by the name it was linked against, on the rpath it carries.
SL=$(mktemp -d "$TARGET/shared.XXXXXX")
if [ "$(uname -s)" = Darwin ]; then
  cp "$TARGET/shared/release/libdaegun.dylib" "$SL/libdaegun.$MAJOR.dylib"
  ln -s "libdaegun.$MAJOR.dylib" "$SL/libdaegun.dylib"
else
  cp "$SO" "$SL/libdaegun.so.$MAJOR"
  ln -s "libdaegun.so.$MAJOR" "$SL/libdaegun.so"
fi
if ! cc -std=c11 -Wall -Wextra -Werror -I src/api/c/include src/api/c/tests/roundtrip.c -L"$SL" -ldaegun \
     -Wl,-rpath,"$SL" -o "$SL/rt" >"$SL/log" 2>&1 \
   || ! "$SL/rt" assets/test-fonts/inter/InterVariable.ttf >>"$SL/log" 2>&1; then
  echo "the round trip failed against the shared library"; tail -20 "$SL/log"; rm -rf "$SL"; exit 1
fi
rm -rf "$SL"
echo "named for ABI $MAJOR: $NAME, versioned $ABI with room to rename, on macOS, $SONAME on Linux, needing" \
  "$GLIBC at most; each exports the header's calls alone, and the round trip runs against" \
  "$( [ "$(uname -s)" = Darwin ] && echo the dylib || echo the .so) by that name"

# The C round trip and C++ handles on Linux in a pinned GCC image: ASan checks leaks once a handle never freed is seen
# to fail, and daegun is merged as the README merges it beside another Rust library.
printf 'c and c++, linux . '
if ! docker info >/dev/null 2>&1; then
  echo "no container runtime (brew install colima docker, then colima start)"; exit 1
fi
case "$(docker info --format '{{.Architecture}}')" in
  aarch64|arm64) ARCH=aarch64 ;;
  x86_64|amd64) ARCH=x86_64 ;;
  *) echo "no Linux target for docker's $(docker info --format '{{.Architecture}}')"; exit 1 ;;
esac
if ! rustup target list --installed 2>/dev/null | grep -q "^$ARCH-unknown-linux-gnu$"; then
  echo "no $ARCH Linux target (rustup target add $ARCH-unknown-linux-gnu)"; exit 1
elif ! cargo rustc -q --features capi --crate-type staticlib --target "$ARCH-unknown-linux-gnu" >/dev/null 2>&1; then
  echo "the Linux library does not build"; exit 1
fi
LS=$(mktemp -d "$TARGET/lsan.XXXXXX")
if [ "$(std_links "$ARCH-unknown-linux-gnu" "$LS")" != "$(links Linux)" ]; then
  echo "the README's Linux line links '$(links Linux)', but Rust's standard library asks for" \
    "'$(std_links "$ARCH-unknown-linux-gnu" "$LS")'"; rm -rf "$LS"; exit 1
elif ! rustup target list --installed 2>/dev/null | grep -q "^$ARCH-unknown-linux-musl$"; then
  echo "no $ARCH musl target (rustup target add $ARCH-unknown-linux-musl)"; rm -rf "$LS"; exit 1
elif [ "$(std_links "$ARCH-unknown-linux-musl" "$LS")" != "$(links 'Linux, musl')" ]; then
  echo "the README's musl line links '$(links 'Linux, musl')', but Rust's standard library asks for" \
    "'$(std_links "$ARCH-unknown-linux-musl" "$LS")'"; rm -rf "$LS"; exit 1
fi
printf '%s\n' '#include "daegun.h"' 'int main() { return daegun_path_new() == nullptr; }' >"$LS/leak.cpp"
mkdir -p "$LS/other/src"
printf '%s\n' '[package]' 'name = "other"' 'version = "0.0.0"' 'edition = "2024"' '' '[lib]' \
  'crate-type = ["staticlib"]' '' '[profile.release]' 'lto = true' 'panic = "abort"' 'codegen-units = 1' '' \
  '[workspace]' >"$LS/other/Cargo.toml"
printf '%s\n' '#[unsafe(no_mangle)]' 'pub extern "C" fn other_answer(n: u32) -> u32 {' \
  '    format!("{}", (0..n).sum::<u32>()).len() as u32' '}' >"$LS/other/src/lib.rs"
printf '%s\n' '#include "daegun.h"' 'uint32_t other_answer(uint32_t n);' 'int main(void) {' \
  '    daegun_path *p = daegun_path_new();' '    int ok = p != NULL && other_answer(100) == 4;' \
  '    daegun_path_free(p);' '    return !ok;' '}' >"$LS/beside.c"
printf '%s\n' '#include "daegun.h"' 'int plugin_paths(void) {' '    daegun_path *p = daegun_path_new();' \
  '    daegun_path_free(p);' '    return p != NULL;' '}' >"$LS/plugin.c"
if ! CARGO_TARGET_DIR="$LS/other/target" cargo build -q --release --target "$ARCH-unknown-linux-gnu" \
     --manifest-path "$LS/other/Cargo.toml" >/dev/null 2>&1; then
  echo "the other Linux library does not build"; rm -rf "$LS"; exit 1
fi
cat >"$LS/run.sh" <<'EOF'
set -e
build() {
  g++ -std=c++17 -g -fsanitize=address -fno-omit-frame-pointer -Wall -Wextra -Werror -pedantic \
    -I src/api/c/include "$1" "$LIB" $SYSLIBS -o "$2"
}
build "$DIR/leak.cpp" /tmp/leak
if ASAN_OPTIONS=detect_leaks=1 /tmp/leak >/dev/null 2>&1; then
  echo "a handle never freed went unseen"; exit 1
fi
build src/api/c/tests/handles.cpp /tmp/handles
ASAN_OPTIONS=detect_leaks=1 /tmp/handles
gcc -std=c11 -g -fsanitize=address -fno-omit-frame-pointer -Wall -Wextra -Werror -pedantic -I src/api/c/include \
  src/api/c/tests/roundtrip.c "$LIB" $SYSLIBS -o /tmp/roundtrip
ASAN_OPTIONS=detect_leaks=1 /tmp/roundtrip assets/test-fonts/inter/InterVariable.ttf >/dev/null
gcc -shared -fPIC -std=c11 -Wall -Wextra -Werror -I src/api/c/include "$DIR/plugin.c" "$LIB" $SYSLIBS $EXCLUDE \
  -o /tmp/libplugin.so
if [ -z "$EXCLUDE" ] || nm -D --defined-only /tmp/libplugin.so | grep -E ' (daegun_|rust_eh_personality)'; then
  echo "a shared library linking daegun with the README's '$EXCLUDE' exports daegun's symbols"; exit 1
fi
mkdir /tmp/merged
cp "$LIB" /tmp/merged/libdaegun.a
(cd /tmp/merged && eval "$MERGE" && eval "$ARCHIVE")
if nm -g --defined-only /tmp/merged/libdaegun.a | grep -E ' [A-TV-Z] ' | grep -v ' daegun_'; then
  echo "the merged library exports more than daegun's calls"; exit 1
fi
gcc -std=c11 -Wall -Wextra -Werror -I src/api/c/include "$DIR/beside.c" /tmp/merged/libdaegun.a "$OTHER" \
  $SYSLIBS -o /tmp/beside
/tmp/beside
EOF
MERGE=$(sed -nE "s/^(ld -r --whole-archive .*)$/\1/p" README.md)
ARCHIVE=$(sed -nE "s/^(rm libdaegun\.a && ar rcs .*)$/\1/p" README.md)
# Each container under --init: a program that is its PID 1 ignores its own abort and hangs the step.
if ! docker run --rm --init -v "$PWD:/src" -v "$TARGET:/target" -w /src -e DIR="/target/${LS##*/}" \
     -e LIB="/target/$ARCH-unknown-linux-gnu/debug/libdaegun.a" -e SYSLIBS="$(links Linux)" -e MERGE="$MERGE" \
     -e ARCHIVE="$ARCHIVE" -e EXCLUDE="$(sed -nE 's/.*`(-Wl,--exclude-libs,[^`]*)`.*/\1/p' README.md)" \
     -e OTHER="/target/${LS##*/}/other/target/$ARCH-unknown-linux-gnu/release/libother.a" gcc:15.3.0 \
     sh "/target/${LS##*/}/run.sh" >"$LS/log" 2>&1; then
  echo "the C round trip, the C++ handles or the merged library failed on Linux"; head -30 "$LS/log"; rm -rf "$LS"
  exit 1
fi
rm -rf "$LS"
echo "the round trip and the C++ handles clean under asan with leaks on Linux $ARCH, GCC 15, once a handle never" \
  "freed fails it, linking what the README's Linux line names, as Rust's standard library asks; daegun merged as" \
  "the README says exports its calls alone and links beside another Rust library"

# cl itself, under Wine. At code page 949 a byte outside ASCII in a header is C4819, an error under /WX.
printf 'msvc ............. '
WIN=x86_64-pc-windows-msvc
CL=""
for c in "$(command -v cl || true)" "$HOME/opt/msvc/bin/x64/cl" /opt/msvc/bin/x64/cl; do
  if [ -n "$c" ] && [ -x "$c" ]; then
    CL=$c
    break
  fi
done
if [ -z "$CL" ] || ! command -v wine >/dev/null 2>&1; then
  echo "no cl under Wine (github.com/mstorsjo/msvc-wine, installed to ~/opt/msvc, with wine on PATH)"; exit 1
elif ! rustup target list --installed | grep -q "^$WIN\$"; then
  echo "no $WIN target (rustup target add $WIN)"; exit 1
elif ! cargo rustc --features capi --crate-type staticlib --target "$WIN" >/dev/null 2>&1; then
  echo "the C ABI does not build for $WIN"
  cargo rustc --features capi --crate-type staticlib --target "$WIN" 2>&1 | tail -20; exit 1
fi
msvc() { env LC_ALL=ko_KR.UTF-8 WINEDEBUG=-all "$CL" /nologo "$@"; }
win() { env WINEDEBUG=-all MVK_CONFIG_LOG_LEVEL=0 wine "$@"; }
CI=src/api/c/include
CW=src/api/c/tests
FONT=assets/test-fonts/inter/InterVariable.ttf
LIB="$TARGET/$WIN/debug/daegun.lib $(links Windows)"
CT=$(mktemp -d)
if [ "$(std_links "$WIN" "$CT")" != "$(links Windows)" ]; then
  echo "the README's Windows line links '$(links Windows)', but Rust's standard library asks for" \
    "'$(std_links "$WIN" "$CT")'"; rm -rf "$CT"; exit 1
fi
# Without code page 949 the header's build can't catch a byte outside ASCII, so a probe holding one must fail.
printf '/* \342\200\223 */\nint main(void) { return 0; }\n' >"$CT/probe.c"
if msvc /W4 /WX /Zs "$CT/probe.c" >"$CT/cl.log" 2>&1 || ! grep -q C4819 "$CT/cl.log"; then
  echo "cl is not at code page 949 (LC_ALL=ko_KR.UTF-8 has to reach Wine)"; rm -rf "$CT"; exit 1
fi
python3 -I scripts/tools/c-api/null-calls.py >"$CT/null_calls.c"
printf '#include "daegun.h"\nint main(void) { return 0; }\n' >"$CT/all.cpp"
if ! msvc /std:c11 /W4 /WX /MD /I "$CI" "$CW/roundtrip.c" $LIB "/Fe$CT/rt-c.exe" "/Fo$CT/" >"$CT/cl.log" 2>&1 \
   || ! msvc /std:c11 /W4 /WX /MD /I "$CI" "$CT/null_calls.c" $LIB "/Fe$CT/nc.exe" "/Fo$CT/" >>"$CT/cl.log" 2>&1 \
   || ! msvc /std:c11 /W4 /WX /MD /I "$CI" "$CW/readme.c" $LIB "/Fe$CT/readme.exe" "/Fo$CT/" >>"$CT/cl.log" 2>&1 \
   || ! msvc /std:c11 /W4 /WX /MD /I "$CI" "$CW/latency.c" $LIB "/Fe$CT/latency.exe" "/Fo$CT/" \
        >>"$CT/cl.log" 2>&1 \
   || ! msvc /std:c++17 /EHsc /W4 /WX /MD /I "$CI" "$CW/handles.cpp" $LIB "/Fe$CT/handles.exe" "/Fo$CT/" \
        >>"$CT/cl.log" 2>&1 \
   || ! msvc /W4 /WX /MD /I "$CI" "$CW/roundtrip.c" $LIB "/Fe$CT/rt-plain.exe" "/Fo$CT/" >>"$CT/cl.log" 2>&1 \
   || ! msvc /Zp4 /W4 /WX /MD /I "$CI" "$CW/roundtrip.c" $LIB "/Fe$CT/rt-zp4.exe" "/Fo$CT/" >>"$CT/cl.log" 2>&1 \
   || ! msvc /std:c17 /W4 /WX /I "$CI" /Zs "$CW/roundtrip.c" "$CT/null_calls.c" >>"$CT/cl.log" 2>&1 \
   || ! msvc /std:clatest /W4 /WX /I "$CI" /Zs "$CW/roundtrip.c" >>"$CT/cl.log" 2>&1 \
   || ! msvc /std:c++14 /EHsc /W4 /WX /I "$CI" /Zs "$CT/all.cpp" >>"$CT/cl.log" 2>&1 \
   || ! msvc /std:c++latest /EHsc /W4 /WX /I "$CI" /Zs "$CW/handles.cpp" >>"$CT/cl.log" 2>&1; then
  echo "the C headers or their tests do not compile: cl"; grep -E "error|warning" "$CT/cl.log" | head -20
  rm -rf "$CT"; exit 1
fi
# A dropped status warns in C++17 and C23, and in C before it under /analyze.
printf '%s\n' '#include "daegun.h"' 'int main(void) {' '    daegun_font *f = 0;' '    daegun_font_open(0, 0, &f);' \
  '    return 0;' '}' >"$CT/drop.c"
cp "$CT/drop.c" "$CT/drop.cpp"
if msvc /std:c++17 /W4 /WX /I "$CI" /c "/Fo$CT/" "$CT/drop.cpp" >"$CT/cl.log" 2>&1 || ! grep -q C4834 "$CT/cl.log" \
   || msvc /std:clatest /W4 /WX /I "$CI" /c "/Fo$CT/" "$CT/drop.c" >"$CT/cl.log" 2>&1 \
   || ! grep -q C4834 "$CT/cl.log" \
   || msvc /std:c11 /analyze /W4 /WX /I "$CI" /c "/Fo$CT/" "$CT/drop.c" >"$CT/cl.log" 2>&1 \
   || ! grep -q C6031 "$CT/cl.log"; then
  echo "cl does not warn on a dropped status"; head -10 "$CT/cl.log"; rm -rf "$CT"; exit 1
fi
# An array shorter than the length passed warns under /analyze, read and written.
printf '%s\n' '#include "daegun.h"' 'int f(const daegun_font *font) {' '    uint32_t gids[2] = {0};' \
  '    double out[2];' '    return daegun_font_advances(font, gids, 4, 0, 0, out);' '}' >"$CT/short.c"
if ! msvc /std:c11 /analyze /W4 /I "$CI" /c "/Fo$CT/" "$CT/short.c" >"$CT/cl.log" 2>&1 \
   || ! grep -q C6385 "$CT/cl.log" || ! grep -q C6386 "$CT/cl.log"; then
  echo "cl /analyze reads no array length"; head -10 "$CT/cl.log"; rm -rf "$CT"; exit 1
fi
# An allocator that may answer NULL warns under /analyze where its answer isn't checked.
printf '%s\n' '#include "daegun.h"' 'uint8_t f(void) {' '    uint8_t *buffer = daegun_font_buffer_new(4);' \
  '    return buffer[0];' '}' >"$CT/maybe.c"
if ! msvc /std:c11 /analyze /W4 /I "$CI" /c "/Fo$CT/" "$CT/maybe.c" >"$CT/cl.log" 2>&1 \
   || ! grep -q C6011 "$CT/cl.log"; then
  echo "cl /analyze isn't told daegun_font_buffer_new may answer NULL"; head -10 "$CT/cl.log"; rm -rf "$CT"; exit 1
fi
# Under /Gv, which makes an unmarked function __vectorcall, each call is DAEGUN_CALL, so a program links and runs,
# and a callback marked DAEGUN_CALLBACK goes in a pen where an unmarked one is refused rather than called wrong.
for mark in DAEGUN_CALLBACK ""; do
  printf '%s\n' '#include "daegun.h"' \
    "static void $mark to(void *u, float x, float y) { *(int *)u += 1; (void)x; (void)y; }" \
    'int __cdecl main(void) {' '    int n = 0;' '    daegun_pen pen = { to, to, 0, 0, 0, &n };' \
    '    daegun_path *p = daegun_path_new();' \
    '    if (!p || daegun_path_move_to(p, 1, 2) || daegun_path_line_to(p, 3, 4)) return 1;' \
    '    if (daegun_path_replay(p, NULL, &pen)) return 2;' '    daegun_path_free(p);' '    return n == 2 ? 0 : 3;' \
    '}' >"$CT/gv.c"
  if [ -n "$mark" ]; then
    if ! msvc /std:c11 /Gv /W4 /WX /MD /I "$CI" "$CT/gv.c" $LIB "/Fe$CT/gv.exe" "/Fo$CT/" >"$CT/cl.log" 2>&1 \
       || ! OUT=$(win "$CT/gv.exe" 2>&1); then
      echo "cl /Gv did not link and run a call with a marked callback"; head -10 "$CT/cl.log"; printf '%s\n' "$OUT"
      rm -rf "$CT"; exit 1
    fi
  elif msvc /std:c11 /Gv /W4 /WX /I "$CI" /c "/Fo$CT/" "$CT/gv.c" >"$CT/cl.log" 2>&1 || ! grep -q C2440 "$CT/cl.log"
  then
    echo "cl /Gv took an unmarked callback"; head -10 "$CT/cl.log"; rm -rf "$CT"; exit 1
  fi
done
if ! OUT=$(win "$CT/rt-c.exe" "$FONT" 2>&1) || ! OUT=$(win "$CT/rt-plain.exe" "$FONT" 2>&1) \
   || ! OUT=$(win "$CT/rt-zp4.exe" "$FONT" 2>&1) \
   || ! OUT=$(win "$CT/nc.exe" 2>&1) \
   || ! OUT=$(win "$CT/handles.exe" 2>&1) || ! OUT=$(win "$CT/readme.exe" "$FONT" 2>&1); then
  echo "the C round trip, the README's C, C++ handles or NULL calls failed: cl"; printf '%s\n' "$OUT"; rm -rf "$CT"
  exit 1
fi
# daegun-$MAJOR.dll, linked by LLVM's lld-link against msvc-wine's libraries (its link.exe stalls under Wine), exports
# the header's calls alone, and a program built with DAEGUN_DLL imports them through its import library.
MS=$(cd "$(dirname "$CL")/../.." && pwd)
SDK=$(ls -d "$MS"/kits/10/lib/*/ 2>/dev/null | tail -1)
VC=$(ls -d "$MS"/vc/tools/msvc/*/ 2>/dev/null | tail -1)
# A write past daegun_font_buffer_new's size warns under /analyze. clang on an MSVC target, which defines no
# __GNUC__, reads the size, the const calls and the cold ones as GCC and clang elsewhere do.
printf '%s\n' '#include "daegun.h"' 'int main(void) {' '    uint8_t *b = daegun_font_buffer_new(4);' \
  '    if (!b) return 1;' '    b[8] = 1;' '    daegun_font_buffer_free(b, 4);' '    return 0;' '}' >"$CT/past.c"
KINC=$(ls -d "$MS"/kits/10/include/*/ 2>/dev/null | tail -1)
ATTRS=$(clang --target="$WIN" -isystem "${VC}include" -isystem "${KINC}ucrt" -isystem "${KINC}shared" \
  -isystem "${KINC}um" -I "$CI" -fsyntax-only -Xclang -ast-dump "$CT/past.c" 2>&1 \
  | grep -oE 'AllocSizeAttr|ConstAttr|ColdAttr' | sort | uniq -c | awk '{ printf "%s %s ", $1, $2 }')
# Counted from the headers, so a call marked later is checked with the rest.
WANT=$(for m in ALLOC_SIZE:AllocSizeAttr COLD:ColdAttr CONST:ConstAttr; do
  printf '%s %s ' "$(grep -hvE '^[[:space:]]*(#[[:space:]]*define|/?\*)' "$CI"/daegun.h "$CI"/daegun/*.h \
    | grep -ow "DAEGUN_${m%%:*}" | wc -l | tr -d ' ')" "${m#*:}"
done)
if ! msvc /std:c11 /analyze /W4 /I "$CI" /c "/Fo$CT/" "$CT/past.c" >"$CT/cl.log" 2>&1 \
   || ! grep -q C6386 "$CT/cl.log"; then
  echo "cl /analyze reads no allocator size"; head -10 "$CT/cl.log"; rm -rf "$CT"; exit 1
elif [ "$ATTRS" != "$WANT" ]; then
  echo "clang on $WIN reads '$ATTRS' of the headers' '$WANT'"; rm -rf "$CT"; exit 1
fi
# Built as the README builds it, so it imports no VC runtime, which a machine without the redistributable lacks.
DLLFLAGS=$(python3 -I -c 'import re; t = re.sub(r"\\\n\s*", "", open("README.md").read())
m = re.search(r"^cargo rustc --release --features capi --crate-type cdylib -- (.*crt-static.*)$", t, re.M)
print(m.group(1) if m else "")')
READOBJ=$(command -v llvm-readobj || echo /opt/homebrew/opt/llvm/bin/llvm-readobj)
if [ -z "$DLLFLAGS" ] || ! cargo rustc --features capi --crate-type cdylib --target "$WIN" -- $DLLFLAGS \
     -C linker=rust-lld -C "link-arg=/LIBPATH:${SDK}um/x64" -C "link-arg=/LIBPATH:${SDK}ucrt/x64" \
     -C "link-arg=/LIBPATH:${VC}lib/x64" >"$CT/dll.log" 2>&1; then
  echo "daegun.dll does not build as the README builds it"; tail -20 "$CT/dll.log"; rm -rf "$CT"; exit 1
elif "$READOBJ" --coff-imports "$TARGET/$WIN/debug/daegun.dll" | grep -iE 'Name: (VCRUNTIME|MSVCP)'; then
  echo "daegun.dll, built as the README builds it, imports a VC runtime"; rm -rf "$CT"; exit 1
fi
# Named by the ABI's major in a folder, its import library made from its exports by the README's line, llvm-lib
# standing for lib: only that name loads.
LIBTOOL=$(command -v llvm-lib || echo /opt/homebrew/opt/llvm/bin/llvm-lib)
LIBLINE=$(sed -nE 's/^`lib (\/def:[^`]*)`\.$/\1/p' README.md)
dll_by_major() {
  { echo "LIBRARY daegun-$MAJOR.dll"; echo EXPORTS; "$READOBJ" --coff-exports "$TARGET/$WIN/debug/daegun.dll" \
      | sed -n 's/^ *Name: //p'; } >"$1/daegun-$MAJOR.def"
  cp "$TARGET/$WIN/debug/daegun.dll" "$1/daegun-$MAJOR.dll"
  (cd "$1" && "$LIBTOOL" $LIBLINE)
}
if [ -z "$LIBLINE" ] || ! dll_by_major "$CT" >"$CT/lib.log" 2>&1; then
  echo "the README's lib line makes no import library for daegun-$MAJOR.dll"; cat "$CT/lib.log"; rm -rf "$CT"; exit 1
fi
abi_out=$(python3 -I scripts/tools/c-api/abi-parity.py "$CT/daegun-$MAJOR.dll.lib") \
  || { echo "issues"; echo "$abi_out"; rm -rf "$CT"; exit 1; }
if ! msvc /std:c11 /W4 /WX /MD /DDAEGUN_DLL /I "$CI" "$CW/roundtrip.c" "$CT/daegun-$MAJOR.dll.lib" \
     "/Fe$CT/rt-dll.exe" "/Fo$CT/" >"$CT/cl.log" 2>&1 || ! OUT=$(win "$CT/rt-dll.exe" "$FONT" 2>&1); then
  echo "the round trip failed against daegun-$MAJOR.dll"; head -20 "$CT/cl.log"; printf '%s\n' "$OUT"; rm -rf "$CT"
  exit 1
fi
# The README's two cl lines as written, the static one on readme.c and the DLL's on the round trip.
ROWCL=$(sed -nE 's/^\| Windows \| `cl ([^`]*)` \|$/\1/p' README.md)
DLLCL=$(sed -nE "s/.*\`cl ([^\`]*daegun-$MAJOR\.dll\.lib)\`.*/\1/p" README.md)
if [ -z "$ROWCL" ] || [ -z "$DLLCL" ] \
   || ! msvc $(echo "$ROWCL" | sed "s#app\.c#$CW/readme.c#; s#daegun\.lib#$TARGET/$WIN/debug/daegun.lib#") \
        /I "$CI" "/Fe$CT/readme-row.exe" "/Fo$CT/" >"$CT/cl.log" 2>&1 \
   || ! msvc $(echo "$DLLCL" | sed "s#app\.c#$CW/roundtrip.c#; s#daegun-$MAJOR\.dll\.lib#$CT/daegun-$MAJOR.dll.lib#") \
        /I "$CI" "/Fe$CT/rt-row.exe" "/Fo$CT/" >>"$CT/cl.log" 2>&1; then
  echo "the README's cl lines do not build as written"; grep -E "error|warning" "$CT/cl.log" | head; rm -rf "$CT"
  exit 1
fi
ver=$(env WINEDEBUG=-all "$CL" 2>&1 | sed -n 's/.*Version \([0-9][0-9.]*\) .*/\1/p' | head -1)
echo "cl $ver: its default C, C11 to the latest C and C++14 to the latest C++ ok at code page 949, a dropped status" \
  "warning in C++17 and C23, a short array's and a write past an allocation under /analyze, clang on $WIN reading" \
  "the size, const and cold marks, a call linked and run under /Gv where an unmarked callback is refused, round" \
  "trip, the README's C, C++ handles and NULL calls run, latency.c builds, linking what the README's Windows line" \
  "names, as Rust's standard library asks; the DLL, built as the README builds it, imports no VC runtime and" \
  "exports the header's calls alone, and the round trip runs against it with DAEGUN_DLL"
rm -rf "$CT"

# MinGW under Wine: the static library built for windows-gnu, linked as the README's MinGW line says, and the DLL
# the MSVC step built, linked through its import library as MinGW users of the release's DLL link it.
printf 'mingw ............ '
GNU=x86_64-pc-windows-gnu
if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "no MinGW (brew install mingw-w64, or apt install mingw-w64)"; exit 1
elif ! rustup target list --installed | grep -q "^$GNU\$"; then
  echo "no $GNU target (rustup target add $GNU)"; exit 1
elif ! cargo rustc --features capi --crate-type staticlib --target "$GNU" >/dev/null 2>&1; then
  echo "the C ABI does not build for $GNU"
  cargo rustc --features capi --crate-type staticlib --target "$GNU" 2>&1 | tail -20; exit 1
fi
CT=$(mktemp -d)
if [ "$(std_links "$GNU" "$CT")" != "$(links MinGW)" ]; then
  echo "the README's MinGW line links '$(links MinGW)', but Rust's standard library asks for" \
    "'$(std_links "$GNU" "$CT")'"; rm -rf "$CT"; exit 1
fi
MG="-std=c11 -Wall -Wextra -Werror -pedantic -I $CI"
# MinGW's own C++ runtime DLLs aren't on Wine's path, so the C++ program carries them.
if ! x86_64-w64-mingw32-gcc $MG "$CW/roundtrip.c" "$TARGET/$GNU/debug/libdaegun.a" $(links MinGW) -o "$CT/rt.exe" \
     >"$CT/cc.log" 2>&1 \
   || ! x86_64-w64-mingw32-gcc $MG "$CW/readme.c" "$TARGET/$GNU/debug/libdaegun.a" $(links MinGW) \
     -o "$CT/readme.exe" >>"$CT/cc.log" 2>&1 \
   || ! x86_64-w64-mingw32-g++ -std=c++17 -Wall -Wextra -Werror -pedantic -static -I "$CI" "$CW/handles.cpp" \
     "$TARGET/$GNU/debug/libdaegun.a" $(links MinGW) -o "$CT/handles.exe" >>"$CT/cc.log" 2>&1 \
   || ! dll_by_major "$CT" >>"$CT/cc.log" 2>&1 \
   || ! x86_64-w64-mingw32-gcc $MG -DDAEGUN_DLL "$CW/roundtrip.c" "$CT/daegun-$MAJOR.dll.lib" \
     -o "$CT/rt-dll.exe" >>"$CT/cc.log" 2>&1; then
  echo "the C tests do not build under MinGW"; head -20 "$CT/cc.log"; rm -rf "$CT"; exit 1
fi
if ! OUT=$(win "$CT/rt.exe" "$FONT" 2>&1) || ! OUT=$(win "$CT/readme.exe" "$FONT" 2>&1) \
   || ! OUT=$(win "$CT/handles.exe" 2>&1) || ! OUT=$(win "$CT/rt-dll.exe" "$FONT" 2>&1); then
  echo "the C tests failed under MinGW"; printf '%s\n' "$OUT"; rm -rf "$CT"; exit 1
fi
rm -rf "$CT"
echo "$(x86_64-w64-mingw32-gcc -dumpfullversion): the round trip, the README's C and the C++ handles run against" \
  "the static library linked as the README's MinGW line says, as Rust's standard library asks, and the round" \
  "trip against daegun-$MAJOR.dll through its import library"

printf 'lib tests asan ... '
if ! rustup run nightly rustc --version >/dev/null 2>&1; then
  echo "no nightly toolchain (rustup toolchain install nightly --profile minimal)"; exit 1
fi
if OUT=$(RUSTFLAGS="-Zsanitizer=address" cargo +nightly test --features capi --lib --target "$HOST" 2>&1); then
  echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 " passed" }'
else
  echo "the library's own tests failed under ASan"
  echo "$OUT" | grep -E "^error|panicked|FAILED|ERROR|SUMMARY|^ +#[0-9]+ " | head -30; exit 1
fi

# The C layer's own tests under Miri, checking each pointer's bounds, provenance and aliasing, and leaks. The corpus
# runs read every font through both sides, hours under Miri, and the rest reach each call.
printf 'c layer, miri .... '
if ! cargo +nightly miri --version >/dev/null 2>&1; then
  echo "no Miri (rustup +nightly component add miri)"; exit 1
fi
if OUT=$(MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test --features capi --lib -- ffi:: \
     --skip c_answers_as_rust_over_the_corpus 2>&1); then
  echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 " passed, nothing undefined and nothing leaked" }'
else
  echo "Miri finds undefined behavior or a leak"
  echo "$OUT" | grep -E "^error|Undefined Behavior|panicked|FAILED|^ +--> " | head -30; exit 1
fi

# What threads meet on, the allowance's draws and the caches' lists, under Miri on a schedule each seed.
printf 'threads, miri .... '
SEEDS=16
if OUT=$(MIRIFLAGS=-Zmiri-many-seeds=0..$SEEDS cargo +nightly miri test --features threading --lib -- \
     daekit::base::sync daekit::cache::lru 2>&1); then
  # The seeds run at once and their lines interleave, so a result is found anywhere in a line.
  echo "$OUT" | grep -o 'test result: ok\. [0-9]*' | awk -v seeds=$SEEDS '{ s = $4 }
    END { print s+0 " tests, each on " seeds " schedules, nothing undefined" }'
else
  echo "Miri finds undefined behavior"
  echo "$OUT" | grep -E "^error|Undefined Behavior|panicked|FAILED|^ +--> " | head -30; exit 1
fi

# The C round trip and C++ handles with both sides under ASan on one runtime, rustc's, so C takes the clang of rustc's
# LLVM. The library keeps no spare handles, so a path used after its free, or freed twice, must be reported.
printf 'c abi, rust asan . '
SYSROOT=$(rustup run nightly rustc --print sysroot)
RT=$(ls "$SYSROOT/lib/rustlib/$HOST/lib/"librustc-nightly_rt.asan.* 2>/dev/null | head -1)
LLVM=$(rustc +nightly -vV | awk '/^LLVM version:/ { split($3, v, "."); print v[1] }')
CLANG=""
for c in /opt/homebrew/opt/llvm@"$LLVM"/bin/clang /opt/homebrew/opt/llvm/bin/clang /usr/lib/llvm-"$LLVM"/bin/clang \
         clang-"$LLVM" clang; do
  if "$c" --version 2>/dev/null | head -1 | grep -qE "clang version $LLVM\."; then
    CLANG=$c
    break
  fi
done
FONT=assets/test-fonts/inter/InterVariable.ttf
CT=$(mktemp -d)
printf '%s\n' '#include "daegun.h"' 'int main(int argc, char **argv) {' '    (void)argv;' \
  '    daegun_path *p = daegun_path_new();' '    daegun_path_free(p);' \
  '    if (argc > 1) daegun_path_free(p); else daegun_path_move_to(p, 1.0f, 2.0f);' '    return 0;' '}' \
  >"$CT/freed.c"
if [ -z "$RT" ]; then
  echo "nightly has no ASan runtime for $HOST"; rm -rf "$CT"; exit 1
elif [ -z "$CLANG" ]; then
  echo "no clang on nightly's LLVM $LLVM (brew install llvm@$LLVM, or apt install clang-$LLVM)"; rm -rf "$CT"; exit 1
elif ! RUSTFLAGS="-Zsanitizer=address --cfg daegun_no_spares" CARGO_TARGET_DIR="$TARGET/asan" cargo +nightly rustc \
       --features capi --crate-type staticlib --target "$HOST" >"$CT/build.log" 2>&1 \
     || ! "$CLANG" -std=c11 -g -fsanitize=address -fno-sanitize-link-runtime -fno-omit-frame-pointer \
       -I src/api/c/include src/api/c/tests/roundtrip.c "$TARGET/asan/$HOST/debug/libdaegun.a" \
       "$RT" -Wl,-rpath,"$(dirname "$RT")" $LINK -o "$CT/rt" >>"$CT/build.log" 2>&1 \
     || ! "$CLANG++" -std=c++17 -g -fsanitize=address -fno-sanitize-link-runtime -fno-omit-frame-pointer \
       -I src/api/c/include src/api/c/tests/handles.cpp "$TARGET/asan/$HOST/debug/libdaegun.a" \
       "$RT" -Wl,-rpath,"$(dirname "$RT")" $LINK -o "$CT/handles" >>"$CT/build.log" 2>&1 \
     || ! "$CLANG" -std=c11 -g -fsanitize=address -fno-sanitize-link-runtime -fno-omit-frame-pointer \
       -I src/api/c/include "$CT/freed.c" "$TARGET/asan/$HOST/debug/libdaegun.a" \
       "$RT" -Wl,-rpath,"$(dirname "$RT")" $LINK -o "$CT/freed" >>"$CT/build.log" 2>&1; then
  echo "does not build"; tail -10 "$CT/build.log"; rm -rf "$CT"; exit 1
elif ! OUT=$(ASAN_OPTIONS=detect_leaks=1 "$CT/rt" "$FONT" 2>&1) \
     || ! OUT=$(ASAN_OPTIONS=detect_leaks=1 "$CT/handles" 2>&1); then
  echo "failed under ASan"; echo "$OUT" | grep -E "FAIL|failed|ERROR|SUMMARY|^ +#[0-9]+ " | head -30
  rm -rf "$CT"; exit 1
# The second free reads the freed path before it frees, so ASan may name either; it exits rather than aborts.
elif ASAN_OPTIONS=abort_on_error=0 "$CT/freed" >"$CT/freed.log" 2>&1 \
     || ! grep -q heap-use-after-free "$CT/freed.log" \
     || ASAN_OPTIONS=abort_on_error=0 "$CT/freed" twice >"$CT/freed.log" 2>&1 \
     || ! grep -qE "double-free|heap-use-after-free" "$CT/freed.log"; then
  echo "a path used after its free, or freed twice, went unseen"; head -5 "$CT/freed.log"; rm -rf "$CT"; exit 1
else
  echo "C round trip and C++ handles clean and leak-free, both sides instrumented (clang $LLVM); a path used" \
    "after its free and one freed twice reported, with no spare handles kept"
fi
rm -rf "$CT"

# One font and one location shared by threads in C, both sides under TSan with std rebuilt instrumented, once a race
# inside the library is seen to be reported: two threads drawing into one path, which rule 5 forbids.
printf 'c abi, rust tsan . '
RT=$(ls "$SYSROOT/lib/rustlib/$HOST/lib/"librustc-nightly_rt.tsan.* 2>/dev/null | head -1)
CT=$(mktemp -d)
tsan() {
  "$CLANG" -std=c11 -g -fsanitize=thread -fno-sanitize-link-runtime -fno-omit-frame-pointer -pthread \
    -I src/api/c/include "$1" "$TARGET/tsan/$HOST/debug/libdaegun.a" "$RT" -Wl,-rpath,"$(dirname "$RT")" $LINK \
    -o "$2" >>"$CT/build.log" 2>&1
}
if [ -z "$RT" ]; then
  echo "nightly has no TSan runtime for $HOST"; rm -rf "$CT"; exit 1
elif ! rustup component list --toolchain nightly --installed | grep -q '^rust-src'; then
  echo "no rust-src for nightly (rustup component add rust-src --toolchain nightly)"; rm -rf "$CT"; exit 1
elif ! RUSTFLAGS="-Zsanitizer=thread" CARGO_TARGET_DIR="$TARGET/tsan" cargo +nightly rustc -Zbuild-std \
       --features capi --crate-type staticlib --target "$HOST" >"$CT/build.log" 2>&1 \
     || ! tsan src/api/c/tests/threads.c "$CT/threads" || ! tsan src/api/c/tests/race.c "$CT/race"; then
  echo "does not build"; tail -10 "$CT/build.log"; rm -rf "$CT"; exit 1
elif TSAN_OPTIONS=halt_on_error=1:abort_on_error=0 "$CT/race" >"$CT/race.log" 2>&1 \
     || ! grep -q "ThreadSanitizer: data race" "$CT/race.log"; then
  echo "a race inside the library went unreported"; head -10 "$CT/race.log"; rm -rf "$CT"; exit 1
elif ! OUT=$("$CT/threads" "$FONT" 2>&1); then
  echo "failed under TSan"; echo "$OUT" | grep -E "FAIL|WARNING|SUMMARY|^ +#[0-9]+ " | head -30; rm -rf "$CT"; exit 1
else
  echo "one font shared by 8 threads, race-free, both sides instrumented; a planted race reported"
fi
rm -rf "$CT"

# The Rust threading tests under TSan, std rebuilt instrumented as for the C step, for paths C's threads don't take.
printf 'threads, tsan .... '
if OUT=$(TSAN_OPTIONS=halt_on_error=1 RUSTFLAGS="-Zsanitizer=thread" CARGO_TARGET_DIR="$TARGET/tsan" \
     cargo +nightly test -Zbuild-std --target "$HOST" --features capi --test api threading:: 2>&1); then
  echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 " tests race-free, std instrumented" }'
else
  echo "a race in the Rust threading tests"
  echo "$OUT" | grep -E "^error|WARNING|SUMMARY|panicked|FAILED|^ +#[0-9]+ " | head -30; exit 1
fi

# MemorySanitizer, Linux's alone: the round trip clean with daegun and Rust's standard library instrumented as the
# README builds them, linked with libgcc first as it says, and reported against the plain library.
printf 'c abi, rust msan . '
MIMG="daegun-msan:$LLVM"
MRUN=$(mktemp -d "$TARGET/msan.XXXXXX")
cat >"$MRUN/run.sh" <<'RUN'
G=$(clang-$LLVM -print-libgcc-file-name)
build() {
  clang-$LLVM -std=c11 -g -fsanitize=memory -fsanitize-memory-track-origins -fno-omit-frame-pointer \
    -I src/api/c/include src/api/c/tests/roundtrip.c "$G" "$1" $LIBS -o "$2" || exit 2
}
build "/target/msan/$ARCH-unknown-linux-gnu/debug/libdaegun.a" /tmp/rt
build "/target/$ARCH-unknown-linux-gnu/debug/libdaegun.a" /tmp/plain
/tmp/rt assets/test-fonts/inter/InterVariable.ttf || exit 3
/tmp/plain assets/test-fonts/inter/InterVariable.ttf >/tmp/plain.log 2>&1 && exit 4
grep -q use-of-uninitialized-value /tmp/plain.log || exit 4
RUN
cat >"$MRUN/Dockerfile" <<DOCKER
FROM gcc:15.3.0
RUN apt-get update && apt-get install -y --no-install-recommends wget gnupg ca-certificates \\
 && wget -qO /etc/apt/trusted.gpg.d/apt.llvm.org.asc https://apt.llvm.org/llvm-snapshot.gpg.key \\
 && echo "deb http://apt.llvm.org/trixie/ llvm-toolchain-trixie-$LLVM main" >/etc/apt/sources.list.d/llvm.list \\
 && apt-get update && apt-get install -y --no-install-recommends clang-$LLVM libclang-rt-$LLVM-dev \\
 && rm -rf /var/lib/apt/lists/*
DOCKER
if ! rustup target list --toolchain nightly --installed | grep -q "^$ARCH-unknown-linux-gnu$"; then
  echo "no $ARCH Linux target for nightly (rustup target add $ARCH-unknown-linux-gnu --toolchain nightly)"
  rm -rf "$MRUN"; exit 1
elif ! grep -qF 'RUSTFLAGS=-Zsanitizer=memory cargo +nightly rustc -Zbuild-std --features capi --crate-type' README.md \
     || ! grep -qF '$(clang -print-libgcc-file-name)' README.md; then
  echo "the README's MemorySanitizer build or link line is gone"; rm -rf "$MRUN"; exit 1
elif ! docker image inspect "$MIMG" >/dev/null 2>&1 \
     && ! docker build -q -t "$MIMG" - <"$MRUN/Dockerfile" >"$MRUN/build.log" 2>&1; then
  echo "no image with clang $LLVM"; tail -10 "$MRUN/build.log"; rm -rf "$MRUN"; exit 1
elif ! RUSTFLAGS="-Zsanitizer=memory -Zsanitizer-memory-track-origins -C debuginfo=line-tables-only" \
       CARGO_TARGET_DIR="$TARGET/msan" cargo +nightly rustc -Zbuild-std --features capi --crate-type staticlib \
       --target "$ARCH-unknown-linux-gnu" >"$MRUN/build.log" 2>&1; then
  echo "does not build"; tail -10 "$MRUN/build.log"; rm -rf "$MRUN"; exit 1
# Docker's seccomp refuses the personality call MSan makes to run a program again without address randomization.
elif ! docker run --rm --init --security-opt seccomp=unconfined -v "$PWD:/src:ro" -v "$TARGET:/target:ro" -w /src \
       -e LLVM="$LLVM" -e ARCH="$ARCH" -e LIBS="$(links Linux)" "$MIMG" sh "/target/${MRUN##*/}/run.sh" \
       >"$MRUN/run.log" 2>&1; then
  echo "the round trip under MSan failed, or the plain library went unreported"
  grep -E "FAIL|WARNING|SUMMARY|error|^ +#[0-9]+ " "$MRUN/run.log" | head -20; rm -rf "$MRUN"; exit 1
else
  echo "C round trip clean, both sides and Rust's std instrumented (clang $LLVM); the plain library reported"
fi
rm -rf "$MRUN"

# latency.c beside its Rust twin, both in release and Rust with threading as C builds it, five runs in turn under each
# of four code layouts: each C row's median gap within api-cost's bound over Rust's, as one link's layout moves a row.
printf 'c latency ........ '
CT=$(mktemp -d)
RUNS=""
# Aligning every block pushes a test branch past its reach on arm64, unless the compiler is told how far that is.
case "$(uname -m)" in arm64|aarch64) REACH=" -C llvm-args=-aarch64-tbz-offset-bits=13" ;; *) REACH="" ;; esac
for layout in "" "-C llvm-args=-align-all-functions=6" "-C llvm-args=-align-loops=64" \
    "-C llvm-args=-align-all-nofallthru-blocks=6$REACH"; do
  export CARGO_TARGET_DIR="$CT/target" RUSTFLAGS="$layout"
  if ! cargo rustc -q --release --features capi --crate-type staticlib >"$CT/build.log" 2>&1 \
     || ! cc -std=c11 -O2 -I src/api/c/include src/api/c/tests/latency.c "$CT/target/release/libdaegun.a" $LINK \
       -o "$CT/latency" >>"$CT/build.log" 2>&1 \
     || ! cargo test -q --release --features threading --test api --no-run >>"$CT/build.log" 2>&1; then
    echo "does not build under '$layout'"; tail -10 "$CT/build.log"; rm -rf "$CT"; exit 1
  fi
  for i in 1 2 3 4 5; do
    at="$CT/$(printf %s "$layout" | tr -cd 'a-z0-9')-$i"
    if ! cargo test -q --release --features threading --test api -- --ignored --exact latency::api_latency \
         --nocapture >"$at.rust" 2>/dev/null || ! "$CT/latency" >"$at.c" 2>&1; then
      echo "a run failed"; cat "$at.c"; rm -rf "$CT"; exit 1
    fi
    RUNS="$RUNS $at.rust $at.c"
  done
  RUNS="$RUNS --"
done
unset CARGO_TARGET_DIR RUSTFLAGS
if ! OUT=$(python3 -I scripts/tools/perf/latency-delta.py $RUNS); then
  echo "$OUT" | sed -n '/^issues/,$p'; rm -rf "$CT"; exit 1
fi
echo "$OUT" | tail -1
rm -rf "$CT"

# One font shared by threads against a font each, which nothing else times: a row far over it queues on a lock.
printf 'threads .......... '
if ! OUT=$(cargo test -q --release --features threading --test api -- --ignored --exact threading::one_font_scales \
    --nocapture 2>&1); then
  echo "FAILED"; echo "$OUT" | grep -E "ns on one font|^error" | head -5; exit 1
fi
echo "$OUT" | grep '^one font at' | sed 's/^one font at //'

# The README's dead-stripping: latency.c, calling few of the calls, links at under half the size, and the round trip
# still runs.
printf 'dead strip ....... '
case "$(uname -s)" in
  Darwin) STRIP_FLAG=-Wl,-dead_strip ;;
  *)      STRIP_FLAG=-Wl,--gc-sections ;;
esac
DS=$(mktemp -d "$TARGET/strip.XXXXXX")
# The release library this step and the next link, as the timed builds above keep theirs apart.
if ! cargo rustc -q --release --features capi --crate-type staticlib >"$DS/log" 2>&1 \
   || ! cc -std=c11 -O2 -I src/api/c/include src/api/c/tests/latency.c "$TARGET/release/libdaegun.a" $LINK \
     -o "$DS/plain" >>"$DS/log" 2>&1 \
   || ! cc -std=c11 -O2 -I src/api/c/include src/api/c/tests/latency.c "$TARGET/release/libdaegun.a" $LINK \
        "$STRIP_FLAG" -o "$DS/stripped" >>"$DS/log" 2>&1 \
   || ! cc -std=c11 -O2 -I src/api/c/include src/api/c/tests/roundtrip.c "$TARGET/release/libdaegun.a" $LINK \
        "$STRIP_FLAG" -o "$DS/rt" >>"$DS/log" 2>&1 \
   || ! "$DS/rt" assets/test-fonts/inter/InterVariable.ttf >>"$DS/log" 2>&1; then
  echo "a dead-stripped build fails"; tail -20 "$DS/log"; rm -rf "$DS"; exit 1
fi
PLAIN=$(wc -c <"$DS/plain" | tr -d ' ')
STRIPPED=$(wc -c <"$DS/stripped" | tr -d ' ')
rm -rf "$DS"
if [ $((STRIPPED * 2)) -ge "$PLAIN" ]; then
  echo "latency.c is $STRIPPED bytes stripped of $PLAIN, not under half as the README says"; exit 1
fi
# The README's sentence counts the calls latency.c makes and the calls the headers declare.
USED=$(grep -oE 'daegun_[a-z0-9_]+\(' src/api/c/tests/latency.c | sort -u | wc -l | tr -d ' ')
ALL=$(cat src/api/c/include/daegun/*.h | grep -cE '^daegun_[a-z0-9_]+\(')
if ! tr '\n' ' ' <README.md | grep -q "one calling $USED of the $ALL comes out"; then
  echo "the README's dead-strip sentence doesn't say latency.c calls $USED of the $ALL"; exit 1
fi
echo "latency.c, calling $USED of the $ALL, links at $((STRIPPED / 1024)) KB with $STRIP_FLAG, from" \
  "$((PLAIN / 1024)) KB, as the README says, and the round trip runs stripped"

# daegun's release library beside another Rust one, built as the README builds daegun: the two clash on Rust's
# standard library, and once merged as the README merges it, they link, and the round trip runs.
printf 'beside rust ...... '
case "$(uname -s)" in
  Darwin) MERGE=$(sed -nE "s/^(cc -r -nostdlib .*)$/\1/p" README.md); GLOBAL='_daegun_' ;;
  *)      MERGE=$(sed -nE "s/^(ld -r --whole-archive .*)$/\1/p" README.md); GLOBAL='daegun_' ;;
esac
ARCHIVE=$(sed -nE "s/^(rm libdaegun\.a && ar rcs .*)$/\1/p" README.md)
if [ -z "$MERGE" ] || [ -z "$ARCHIVE" ]; then
  echo "the README's merge lines are missing"; exit 1
fi
BR=$(mktemp -d "$TARGET/beside.XXXXXX")
mkdir -p "$BR/other/src" "$BR/merged"
printf '%s\n' '[package]' 'name = "other"' 'version = "0.0.0"' 'edition = "2024"' '' '[lib]' \
  'crate-type = ["staticlib"]' '' '[profile.release]' 'lto = true' 'panic = "abort"' 'codegen-units = 1' '' \
  '[workspace]' >"$BR/other/Cargo.toml"
printf '%s\n' '#[unsafe(no_mangle)]' 'pub extern "C" fn other_answer(n: u32) -> u32 {' \
  '    format!("{}", (0..n).sum::<u32>()).len() as u32' '}' >"$BR/other/src/lib.rs"
printf '%s\n' '#include <stdio.h>' '#include "daegun.h"' 'uint32_t other_answer(uint32_t n);' 'int main(void) {' \
  '    daegun_path *p = daegun_path_new();' '    int ok = p != NULL && other_answer(100) == 4;' \
  '    daegun_path_free(p);' '    return !ok;' '}' >"$BR/main.c"
cp "$TARGET/release/libdaegun.a" "$BR/merged/"
OTHER="$BR/other/target/release/libother.a"
if ! CARGO_TARGET_DIR="$BR/other/target" cargo build -q --release --manifest-path "$BR/other/Cargo.toml" \
     >"$BR/log" 2>&1; then
  echo "the other library does not build"; tail -10 "$BR/log"; rm -rf "$BR"; exit 1
elif cc -I src/api/c/include "$BR/main.c" "$TARGET/release/libdaegun.a" "$OTHER" $LINK -o "$BR/two" >/dev/null 2>&1
then
  echo "two Rust libraries link unmerged, so the README's note is out of date"; rm -rf "$BR"; exit 1
elif ! (cd "$BR/merged" && eval "$MERGE" && eval "$ARCHIVE") >"$BR/log" 2>&1; then
  echo "the README's merge fails"; tail -10 "$BR/log"; rm -rf "$BR"; exit 1
fi
STRAYS=$(nm -g "$BR/merged/libdaegun.a" 2>/dev/null | grep -E ' [A-TV-Z] ' | grep -vc " $GLOBAL" || true)
if [ "$STRAYS" != 0 ]; then
  echo "the merged library exports $STRAYS symbols not daegun's"; rm -rf "$BR"; exit 1
elif ! cc -I src/api/c/include "$BR/main.c" "$BR/merged/libdaegun.a" "$OTHER" $LINK -o "$BR/two" >"$BR/log" 2>&1 \
   || ! "$BR/two" \
   || ! cc -std=c11 -I src/api/c/include src/api/c/tests/roundtrip.c "$BR/merged/libdaegun.a" $LINK -o "$BR/rt" \
        >>"$BR/log" 2>&1 \
   || ! "$BR/rt" assets/test-fonts/inter/InterVariable.ttf >>"$BR/log" 2>&1; then
  echo "the merged library fails beside another"; tail -20 "$BR/log"; rm -rf "$BR"; exit 1
fi
rm -rf "$BR"
echo "they clash unmerged; merged as the README says, daegun exports its calls alone, links beside the other, and" \
  "the round trip runs"

printf 'c parity ......... '
if ! sh scripts/tools/c-api/c-parity.sh; then
  exit 1
fi

printf 'c statuses ....... '
if ! python3 -I scripts/tools/c-api/c-statuses.py; then
  exit 1
fi

printf 'c signatures ..... '
if ! python3 -I scripts/tools/c-api/c-signatures.py; then
  exit 1
fi

printf 'c doc comments ... '
if ! python3 -I scripts/tools/c-api/c-doc-comments.py; then
  exit 1
fi

printf 'abi parity ....... '
# Only the library this run built: anything else under target/ may predate the header. One line
# unless it fails, when the whole report shows.
abi_out=$(python3 -I scripts/tools/c-api/abi-parity.py "$TARGET/debug/libdaegun.a") \
  || { echo "issues"; echo "$abi_out"; exit 1; }
echo "$abi_out" | tail -1

# The C ABI on a 32-bit target, where its layout checks must hold without 8-byte pointers. The header too, where
# clang can aim at the target without its system headers.
printf '32-bit c abi ..... '
if ! rustup target list --installed 2>/dev/null | grep -q '^i686-unknown-linux-gnu$'; then
  echo "no i686 target (rustup target add i686-unknown-linux-gnu)"; exit 1
elif ! command -v clang >/dev/null 2>&1; then
  echo "no clang"; exit 1
fi
[ "$(cargo check --target i686-unknown-linux-gnu --features capi 2>&1 | grep -cE '^(warning|error)')" -eq 0 ] \
  || { echo "issues"; cargo check --target i686-unknown-linux-gnu --features capi 2>&1 | tail -20; exit 1; }
printf '#include "daegun.h"\n' | clang --target=i686-unknown-linux-gnu -std=c11 -ffreestanding -Werror \
  -I src/api/c/include -fsyntax-only -x c - || { echo "the headers fail on i686"; exit 1; }
# -mrtd makes an unmarked function stdcall on i386, so each call and callback says cdecl: read from the code, as an
# AST prints a cdecl pointer's type with the default's name.
if printf '%s\n' '#include "daegun.h"' 'int f(daegun_word_segmenter s, const daegun_paint_visitor *v) {' \
     '    v->leave(0);' '    return (int)s(0, "a", 1, 0, 0) + (int)daegun_abi_version();' '}' \
     | clang --target=i686-unknown-linux-gnu -mrtd -std=c11 -ffreestanding -I src/api/c/include -S -emit-llvm -o - \
       -x c - 2>/dev/null | grep -E '^ *(%[0-9]+ = )?(tail )?call |^declare' | grep -q x86_stdcallcc; then
  echo "a daegun call or callback takes -mrtd's stdcall on i386"; exit 1
fi
echo clean

# The checkout at its own path in a container, where the tests find their fonts: Docker on colima binds it there
# empty at times, so it's bound at /src and linked to. The command and its arguments follow.
HERE='mkdir -p "${P%/*}" && ln -s /src "$P" && cd "$P" && exec "$@"'

# Offsets and counts only a 32-bit usize overflows, which no 64-bit run reaches, over the repo's fonts.
printf '32-bit fuzz ...... '
FUZZ32="$TARGET/fuzz32"
if ! (cd scripts/tools/fuzz && CARGO_TARGET_DIR="$FUZZ32" cargo-zigbuild build -q --release \
       --target i686-unknown-linux-gnu.2.28) >"$TARGET/fuzz32.log" 2>&1; then
  echo "does not build (cargo install --locked cargo-zigbuild, brew install zig)"; tail -10 "$TARGET/fuzz32.log"; exit 1
elif ! OUT=$(docker run --rm --init --platform linux/386 -v "$PWD:/src:ro" -e P="$PWD" -v "$FUZZ32:/fuzz:ro" \
       i386/debian:bookworm-slim sh -c "$HERE" sh /fuzz/i686-unknown-linux-gnu/release/daegun-fuzz --count 500 \
       2>&1); then
  echo "FAILED"; echo "$OUT" | head -20; exit 1
fi
echo "$OUT"

# The tests on a 32-bit usize with overflow checks on, where a hostile size overflows as no 64-bit run shows: at
# opt-level 1 and a test thread for each of the container runtime's cores, as emulation is slow.
printf '32-bit tests ..... '
TEST32="$TARGET/test32"
if ! BINS=$(CARGO_TARGET_DIR="$TEST32" CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
       CARGO_PROFILE_RELEASE_OPT_LEVEL=1 CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true \
       CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo-zigbuild test --release --no-run --features capi,dictionaries \
       --target i686-unknown-linux-gnu.2.28 2>&1); then
  echo "does not build"; echo "$BINS" | tail -10; exit 1
fi
PASSED=0
CORES=$(docker info --format '{{.NCPU}}')
for BIN in $(echo "$BINS" | sed -nE 's#.*Executable .*/(i686-unknown-linux-gnu/[^)]*)\)#\1#p'); do
  if ! OUT=$(docker run --rm --init --platform linux/386 -v "$PWD:/src:ro" -e P="$PWD" -v "$TEST32:/t:ro" \
         i386/debian:bookworm-slim sh -c "$HERE" sh "/t/$BIN" --test-threads="$CORES" 2>&1); then
    echo "FAILED"; echo "$OUT" | tail -20; exit 1
  fi
  PASSED=$((PASSED + $(echo "$OUT" | sed -nE 's/^test result: ok\. ([0-9]+) passed.*/\1/p')))
done
[ "$PASSED" -gt 1000 ] || { echo "only $PASSED tests ran"; exit 1; }
echo "$PASSED passed, overflow checks on"

# The rasterizer's bytes, one hash over every kind of image pinned by a test: the host draws them with std and
# without in the runs above, i686 in the last, and here x86_64 Linux and Windows.
printf 'raster bytes ..... '
RX="$TARGET/raster-x"
if ! LBIN=$(CARGO_TARGET_DIR="$RX" CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
       cargo-zigbuild test --release --no-run --lib --target x86_64-unknown-linux-gnu.2.28 2>&1) \
   || ! WBIN=$(CARGO_TARGET_DIR="$RX" CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
       CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc cargo test --release --no-run --lib \
       --target x86_64-pc-windows-gnu 2>&1); then
  echo "the tests don't build for x86_64 Linux or Windows"; printf '%s\n%s\n' "$LBIN" "$WBIN" | tail -10; exit 1
fi
LBIN=$(echo "$LBIN" | sed -nE 's#.*Executable .*/(x86_64-unknown-linux-gnu/[^)]*)\)#\1#p')
WBIN=$(echo "$WBIN" | sed -nE 's#.*Executable .*/(x86_64-pc-windows-gnu/[^)]*)\)#\1#p')
ONE='test result: ok. 1 passed'
if ! OUT=$(docker run --rm --init --platform linux/amd64 -v "$PWD:/src:ro" -e P="$PWD" -v "$RX:/rx:ro" \
       debian:bookworm-slim sh -c "$HERE" sh "/rx/$LBIN" determinism 2>&1) || ! echo "$OUT" | grep -q "$ONE"; then
  echo "x86_64 Linux draws other bytes"; echo "$OUT" | tail -8; exit 1
elif ! OUT=$(win "$RX/$WBIN" determinism 2>&1) || ! echo "$OUT" | grep -q "$ONE"; then
  echo "x86_64 Windows draws other bytes"; echo "$OUT" | tail -8; exit 1
fi
echo "every kind of image the same bytes on aarch64 macOS with std and without, i686 and x86_64 Linux, and" \
  "x86_64 Windows"

# The C package as cargo-c installs it: the headers as they are, the libraries versioned by the C ABI, the notices,
# and a .pc that pkg-config, CMake and Meson find as the README has them, the round trip running through each.
printf 'c package ........ '
for tool in "cargo-cinstall:cargo install --locked cargo-c" "pkg-config:brew install pkgconf" \
    "cmake:brew install cmake" "meson:brew install meson" "ninja:brew install ninja"; do
  if ! command -v "${tool%%:*}" >/dev/null 2>&1; then
    echo "no ${tool%%:*} (${tool#*:})"; exit 1
  fi
done
ABI=$(header_abi)
CP=$(mktemp -d "$TARGET/cpackage.XXXXXX")
if ! cargo cinstall --release --prefix "$CP/prefix" --libdir "$CP/prefix/lib" >"$CP/log" 2>&1; then
  echo "cargo cinstall fails"; tail -20 "$CP/log"; rm -rf "$CP"; exit 1
fi
export PKG_CONFIG_PATH="$CP/prefix/lib/pkgconfig"
case "$(uname -s)" in
  Darwin) SHARED_LIB="$CP/prefix/lib/libdaegun.dylib"; VERSIONED="libdaegun.$ABI.dylib" ;;
  *)      SHARED_LIB="$CP/prefix/lib/libdaegun.so"; VERSIONED="libdaegun.so.$ABI" ;;
esac
if ! diff -r -x .DS_Store src/api/c/include "$CP/prefix/include" >/dev/null; then
  echo "the installed headers differ from src/api/c/include"; rm -rf "$CP"; exit 1
elif [ "$(pkg-config --modversion daegun)" != "$ABI" ] || [ ! -e "$CP/prefix/lib/$VERSIONED" ]; then
  echo "the package is not versioned by the C ABI's $ABI: $(pkg-config --modversion daegun)"; rm -rf "$CP"; exit 1
elif ! cmp -s LICENSE "$CP/prefix/share/doc/daegun/LICENSE" \
     || ! cmp -s CREDITS.txt "$CP/prefix/share/doc/daegun/CREDITS.txt"; then
  echo "the notices aren't installed in share/doc/daegun"; rm -rf "$CP"; exit 1
fi
abi_out=$(python3 -I scripts/tools/c-api/abi-parity.py "$SHARED_LIB") \
  || { echo "issues"; echo "$abi_out"; rm -rf "$CP"; exit 1; }
FONT=assets/test-fonts/inter/InterVariable.ttf
mkdir -p "$CP/cmake" "$CP/meson"
cp src/api/c/tests/roundtrip.c "$CP/cmake/app.c"
cp src/api/c/tests/roundtrip.c "$CP/meson/app.c"
{ printf '%s\n' 'cmake_minimum_required(VERSION 3.16)' 'project(app C)' 'add_executable(app app.c)'
  awk '/^```cmake$/ { f = 1; next } /^```$/ { f = 0 } f' README.md; } >"$CP/cmake/CMakeLists.txt"
{ printf '%s\n' "project('app', 'c')"; awk '/^```meson$/ { f = 1; next } /^```$/ { f = 0 } f' README.md; } \
  >"$CP/meson/meson.build"
if ! cc -std=c11 -Wall -Wextra -Werror $(pkg-config --cflags daegun) src/api/c/tests/roundtrip.c \
     $(pkg-config --libs daegun) -Wl,-rpath,"$CP/prefix/lib" -o "$CP/shared" >>"$CP/log" 2>&1 \
   || ! "$CP/shared" "$FONT" >>"$CP/log" 2>&1 \
   || ! cc -std=c11 $(pkg-config --cflags daegun) src/api/c/tests/roundtrip.c "$CP/prefix/lib/libdaegun.a" \
        $(pkg-config --static --libs-only-l daegun | sed 's/-ldaegun//') -o "$CP/static" >>"$CP/log" 2>&1 \
   || ! "$CP/static" "$FONT" >>"$CP/log" 2>&1; then
  echo "the round trip fails through pkg-config"; tail -20 "$CP/log"; rm -rf "$CP"; exit 1
elif ! cmake -S "$CP/cmake" -B "$CP/cmake/build" -G Ninja >>"$CP/log" 2>&1 \
   || ! cmake --build "$CP/cmake/build" >>"$CP/log" 2>&1 || ! "$CP/cmake/build/app" "$FONT" >>"$CP/log" 2>&1; then
  echo "the README's CMake lines fail"; tail -20 "$CP/log"; rm -rf "$CP"; exit 1
elif ! meson setup "$CP/meson/build" "$CP/meson" >>"$CP/log" 2>&1 || ! meson compile -C "$CP/meson/build" \
     >>"$CP/log" 2>&1 || ! "$CP/meson/build/app" "$FONT" >>"$CP/log" 2>&1; then
  echo "the README's Meson line fails"; tail -20 "$CP/log"; rm -rf "$CP"; exit 1
fi
# The README's install as written, staged as yourself and copied as root, but for root's /usr/local: the same files.
STAGE=$(sed -nE 's/^(cargo cinstall --release --prefix \/usr\/local --destdir) stage$/\1/p' README.md)
COPY=$(sed -nE 's/^sudo (cp -R) stage\/usr\/local\/\. \/usr\/local\/$/\1/p' README.md)
mkdir -p "$CP/root/usr/local"
if [ -z "$STAGE" ] || [ -z "$COPY" ] || ! $STAGE "$CP/stage" >>"$CP/log" 2>&1 \
   || ! $COPY "$CP/stage/usr/local/." "$CP/root/usr/local/" \
   || [ "$(cd "$CP/root/usr/local" && find . | sort)" != "$(cd "$CP/prefix" && find . | sort)" ] \
   || ! grep -qx 'prefix=/usr/local' "$CP/root/usr/local/lib/pkgconfig/daegun.pc"; then
  echo "the README's install, staged and copied, lays down other files"; tail -20 "$CP/log"; rm -rf "$CP"; exit 1
fi
unset PKG_CONFIG_PATH
rm -rf "$CP"
echo "cargo cinstall lays down the headers as they are, $VERSIONED, a .pc at $ABI and the notices; the round trip" \
  "runs through pkg-config shared and static, the README's CMake and its Meson; the README's install, staged and" \
  "copied, lays down the same"

# iOS: the static library the README's macOS line covers, linked into a program through the iPhoneOS SDK.
printf 'ios .............. '
if ! rustup target list --installed 2>/dev/null | grep -q '^aarch64-apple-ios$'; then
  echo "no iOS target (rustup target add aarch64-apple-ios)"; exit 1
elif ! xcrun --sdk iphoneos --show-sdk-path >/dev/null 2>&1; then
  echo "no iPhoneOS SDK (install Xcode)"; exit 1
elif ! cargo rustc -q --features capi --crate-type staticlib --target aarch64-apple-ios >/dev/null 2>&1; then
  echo "the iOS library does not build"; exit 1
fi
IO=$(mktemp -d "$TARGET/ios.XXXXXX")
if [ "$(std_links aarch64-apple-ios "$IO")" != "$(links 'macOS, iOS')" ]; then
  echo "the README's iOS line links '$(links 'macOS, iOS')', but Rust's standard library asks for" \
    "'$(std_links aarch64-apple-ios "$IO")'"; rm -rf "$IO"; exit 1
fi
if ! xcrun --sdk iphoneos clang -target arm64-apple-ios15.0 -std=c11 -Wall -Wextra -Werror -I src/api/c/include \
     src/api/c/tests/roundtrip.c "$TARGET/aarch64-apple-ios/debug/libdaegun.a" -o "$IO/rt" >"$IO/log" 2>&1; then
  echo "the round trip does not link for iOS"; tail -20 "$IO/log"; rm -rf "$IO"; exit 1
fi
# The round trip and the README's C run in a Simulator made for the step, which goes when it ends.
SIMULATOR=$(xcrun simctl list -j runtimes available 2>/dev/null | python3 -c '
import json, sys
ios = [r for r in json.load(sys.stdin)["runtimes"] if r["platform"] == "iOS"]
phones = [d["identifier"] for d in ios[-1]["supportedDeviceTypes"] if d["productFamily"] == "iPhone"] if ios else []
print(phones[0], ios[-1]["identifier"]) if phones else None')
if ! rustup target list --installed 2>/dev/null | grep -q '^aarch64-apple-ios-sim$'; then
  echo "no iOS Simulator target (rustup target add aarch64-apple-ios-sim)"; rm -rf "$IO"; exit 1
elif [ -z "$SIMULATOR" ]; then
  echo "no iOS Simulator runtime (xcodebuild -downloadPlatform iOS)"; rm -rf "$IO"; exit 1
elif ! cargo rustc -q --features capi --crate-type staticlib --target aarch64-apple-ios-sim >/dev/null 2>&1; then
  echo "the iOS Simulator library does not build"; rm -rf "$IO"; exit 1
fi
for t in roundtrip readme; do
  if ! xcrun --sdk iphonesimulator clang -target arm64-apple-ios15.0-simulator -std=c11 -Wall -Wextra -Werror \
       -I src/api/c/include "src/api/c/tests/$t.c" "$TARGET/aarch64-apple-ios-sim/debug/libdaegun.a" -o "$IO/$t" \
       >"$IO/log" 2>&1; then
    echo "$t.c does not link for the iOS Simulator"; tail -20 "$IO/log"; rm -rf "$IO"; exit 1
  fi
done
if ! SIM=$(xcrun simctl create daegun-gate $SIMULATOR 2>"$IO/log"); then
  echo "no Simulator could be made"; tail -5 "$IO/log"; rm -rf "$IO"; exit 1
fi
if ! xcrun simctl boot "$SIM" >"$IO/log" 2>&1 || ! xcrun simctl bootstatus "$SIM" >>"$IO/log" 2>&1 \
   || ! SIMCTL_CHILD_DAEGUN_ROOT="$PWD" xcrun simctl spawn "$SIM" "$IO/roundtrip" >>"$IO/log" 2>&1 \
   || ! xcrun simctl spawn "$SIM" "$IO/readme" "$PWD/assets/test-fonts/inter/InterVariable.ttf" >>"$IO/log" 2>&1; then
  echo "the round trip or the README's C failed in the iOS Simulator"; tail -20 "$IO/log"
  xcrun simctl shutdown "$SIM" >/dev/null 2>&1 || true; xcrun simctl delete "$SIM"; rm -rf "$IO"; exit 1
fi
xcrun simctl shutdown "$SIM"
xcrun simctl delete "$SIM"
rm -rf "$IO"
abi_out=$(python3 -I scripts/tools/c-api/abi-parity.py "$TARGET/aarch64-apple-ios/debug/libdaegun.a" \
  "$TARGET/aarch64-apple-ios-sim/debug/libdaegun.a") || { echo "issues"; echo "$abi_out"; exit 1; }
echo "the round trip links for arm64 iOS 15 as the README's line says, and runs with the README's C in the" \
  "Simulator; both libraries export the header's calls"

# What crates.io gets: no tests and no benchmark notes, built from the .crate itself, then checked with every
# feature and documented as docs.rs documents it, on nightly. Cargo's note on each test target it drops is expected.
printf 'package .......... '
# No dependency of any kind, under every feature: a .lock or metadata naming another package fails.
DEPS=$(cargo metadata --format-version 1 --all-features 2>/dev/null | python3 -c \
  'import json, sys; print(" ".join(sorted({p["name"] for p in json.load(sys.stdin)["packages"]} - {"daegun"})))')
if [ -n "$DEPS" ] || [ "$(grep -c '^\[\[package\]\]$' Cargo.lock)" != 1 ]; then
  echo "the crate depends on something: ${DEPS:-see Cargo.lock}"; exit 1
fi
if ! OUT=$(cargo package --allow-dirty 2>&1); then
  echo "the crate does not package"; echo "$OUT" | tail -20; exit 1
fi
if echo "$OUT" | grep '^warning' | grep -v '^warning: ignoring test `[a-z]*` as `src/[a-z/]*/tests/'; then
  echo "cargo package warns"; exit 1
fi
FILES=$(cargo package --list --allow-dirty 2>/dev/null)
if echo "$FILES" | grep -E '(^|/)tests/|^benchmark\.md$|^(tasks|assets|scripts)/'; then
  echo "the crate carries files it shouldn't"; exit 1
fi
PACKED=$(echo "$FILES" | wc -l | tr -d ' ')
if ! grep -q "The crate ships without its tests: $PACKED files" tasks/book/2.0.0-changelog.md; then
  echo "the changelog's count of the crate's files isn't its $PACKED"; exit 1
fi
PKG="$TARGET/package/daegun-$(sed -nE 's/^version = "(.*)"$/\1/p' Cargo.toml | head -1)"
if ! CARGO_TARGET_DIR="$TARGET/package-check" cargo check -q --all-features --manifest-path "$PKG/Cargo.toml" \
     >/dev/null 2>&1; then
  echo "the packaged crate does not build with every feature"; exit 1
elif ! RUSTDOCFLAGS="--cfg docsrs -D warnings" CARGO_TARGET_DIR="$TARGET/package-check" \
     cargo +nightly doc -q --no-deps --all-features --manifest-path "$PKG/Cargo.toml" >/dev/null 2>&1; then
  echo "the packaged crate's docs fail as docs.rs builds them (rustup toolchain install nightly)"; exit 1
elif ! OUT=$(CARGO_TARGET_DIR="$TARGET/package-check" cargo test -q --doc --manifest-path "$PKG/Cargo.toml" 2>&1); then
  echo "the packaged crate's doc tests, its tests, fail"; echo "$OUT" | tail -20; exit 1
fi
echo "no dependencies; $PACKED files, no tests but doc tests, which pass; builds from" \
  "the .crate, with every feature, and its docs as docs.rs builds them"

# benchmark.md's operation rows, each one of daegun's latency tests.
printf 'bench rows ....... '
if ! OUT=$(python3 -I scripts/tools/perf/bench-rows.py); then
  echo "$OUT"; exit 1
fi
echo "$OUT"

printf 'rust surface ..... '
python3 -I scripts/tools/rust-api/rust-surface.py || exit 1

printf 'MSRV 1.99.0 ...... '
if ! rustup run 1.99.0 rustc --version >/dev/null 2>&1; then
  echo "no 1.99.0 toolchain (rustup toolchain install 1.99.0 --profile minimal)"; exit 1
fi
each_feature_set rustup run 1.99.0 cargo check --all-targets || exit 1
echo clean

echo "GATE GREEN ($N tests, $TN with threading, $CN with capi)"
