#!/bin/sh
set -e
cd "$(dirname "$0")/../../.."

# Where cargo builds, which CARGO_TARGET_DIR or a config can move away from ./target.
TARGET=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')
case "$(uname -s)" in
  Darwin) LINK="" ;;
  *)      LINK="-lgcc_s -lutil -lrt -lpthread -lm -ldl" ;;
esac

printf 'release build .... '
cargo build --release >/dev/null 2>&1 || { echo "FAILED"; cargo build --release 2>&1 | grep -E "^error" -A6 | head -30; exit 1; }
echo ok

printf 'debug build ...... '
cargo build >/dev/null 2>&1 || { echo "FAILED"; cargo build 2>&1 | grep -E "^error" -A6 | head -30; exit 1; }
echo ok

printf 'tests ............ '
OUT=$(cargo test --tests 2>&1) || { echo "FAILED"; echo "$OUT" | grep -E "^(error|failures:)" -A8 | head -40; exit 1; }
N=$(echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 }')
[ "$N" -eq 0 ] && echo "0 (the suite is empty)" || echo "$N passed"

printf 'clippy ........... '
[ "$(cargo clippy --all-targets 2>&1 | grep -cE '^(warning|error)')" -eq 0 ] && echo clean || { echo "issues"; exit 1; }

printf 'rustdoc links .... '
if [ "$(cargo doc -p daegun --no-deps 2>&1 | grep -cE '^(warning|error)')" -ne 0 ]; then
  echo "issues"; cargo doc -p daegun --no-deps 2>&1 | grep -E '^(warning|error)' | head -5; exit 1
fi
echo clean

printf 'fuzz ............. '
if OUT=$(sh scripts/tools/fuzz/run.sh 2>&1); then
  echo "$OUT"
else
  echo "FAILED"; echo "$OUT" | head -20; exit 1
fi

printf 'feature threading '
[ "$(cargo build --features threading 2>&1 | grep -cE '^(warning|error)')" -eq 0 ] && echo clean || { echo "issues"; exit 1; }

printf 'threading tests .. '
OUT=$(cargo test --features threading --tests 2>&1) || {
  echo "FAILED"; echo "$OUT" | grep -E "^(error|failures:)" -A8 | head -40; exit 1; }
TN=$(echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 }')
echo "$TN passed"

printf 'capi tests ....... '
OUT=$(cargo test --features capi --tests 2>&1) || {
  echo "FAILED"; echo "$OUT" | grep -E "^(error|failures:)" -A8 | head -40; exit 1; }
CN=$(echo "$OUT" | awk '/^test result: ok\./ { s += $4 } END { print s+0 }')
echo "$CN passed"

printf 'no-default ....... '
[ "$(cargo build --no-default-features 2>&1 | grep -cE '^(warning|error)')" -eq 0 ] && echo clean || { echo "issues"; exit 1; }

printf 'capi + no-default '
[ "$(cargo clippy --all-targets --no-default-features --features capi 2>&1 | grep -cE '^(warning|error)')" -eq 0 ] \
  && echo clean || { echo "issues"; cargo clippy --all-targets --no-default-features --features capi 2>&1 | grep -E '^(warning|error)' -A6 | head -20; exit 1; }

printf 'c abi ............ '
if ! command -v cc >/dev/null 2>&1; then
  echo "skipped (no C compiler)"
elif ! cargo rustc --features capi --crate-type staticlib >/dev/null 2>&1; then
  echo "the C ABI does not build"; cargo rustc --features capi --crate-type staticlib 2>&1 | tail -20; exit 1
else
  CW=src/c-wrapper
  if ! cargo test --features capi --lib --quiet >/dev/null 2>&1; then
    echo "the C ABI's own tests failed"; cargo test --features capi --lib 2>&1 | tail -20; exit 1
  fi
  CT=$(mktemp -d)
  if ! cc -std=c11 -Wall -Wextra -Werror -I "$CW" "$CW/tests/roundtrip.c" \
       "$TARGET/debug/libdaegun.a" $LINK -o "$CT/rt" 2>"$CT/cc.log"; then
    echo "header or test does not compile"; cat "$CT/cc.log"; rm -rf "$CT"; exit 1
  fi
  if ! OUT=$("$CT/rt" assets/test-fonts/inter/InterVariable.ttf 2>&1); then
    echo "round trip failed"; echo "$OUT"; rm -rf "$CT"; exit 1
  fi
  # UBSan reports and carries on unless told not to, which would leave the exit status clean.
  if cc -std=c11 -g -fsanitize=address,undefined -fno-sanitize-recover=undefined -fno-omit-frame-pointer \
       -I "$CW" "$CW/tests/roundtrip.c" "$TARGET/debug/libdaegun.a" $LINK -o "$CT/rt-san" 2>/dev/null; then
    if ! OUT=$("$CT/rt-san" assets/test-fonts/inter/InterVariable.ttf 2>&1); then
      echo "failed under asan+ubsan"
      echo "$OUT" | grep -E "FAIL|failed|ERROR|SUMMARY|runtime error|^ +#[0-9]+ " | head -30; rm -rf "$CT"; exit 1
    fi
    echo "round trip ok, clean under asan+ubsan"
  else
    echo "round trip ok (no sanitizer available)"
  fi
  rm -rf "$CT"
fi

if rustup run nightly rustc --version >/dev/null 2>&1; then
  HOST=$(rustc -vV | awk '/^host:/ {print $2}')
  printf 'lib tests asan ... '
  if OUT=$(RUSTFLAGS="-Zsanitizer=address" cargo +nightly test --features capi --lib --target "$HOST" 2>&1); then
    echo "clean"
  else
    echo "the library's own tests failed under ASan"
    echo "$OUT" | grep -E "^error|panicked|FAILED|ERROR|SUMMARY|^ +#[0-9]+ " | head -30; exit 1
  fi

  # The C round trip with daegun instrumented too: -fsanitize sees only the C side, so a use-after-free
  # inside the library passes it. Linked to rustc's own runtime, which matches the LLVM that built it.
  printf 'c abi, rust asan . '
  RT=$(ls "$(rustup run nightly rustc --print sysroot)/lib/rustlib/$HOST/lib/"librustc-nightly_rt.asan.* 2>/dev/null | head -1)
  CT=$(mktemp -d)
  if [ -z "$RT" ] || ! command -v cc >/dev/null 2>&1; then
    echo "skipped (no sanitizer runtime or C compiler)"
  elif ! RUSTFLAGS="-Zsanitizer=address" CARGO_TARGET_DIR="$TARGET/asan" cargo +nightly rustc --features capi \
         --crate-type staticlib --target "$HOST" >"$CT/build.log" 2>&1 \
       || ! cc -std=c11 -g -I src/c-wrapper src/c-wrapper/tests/roundtrip.c "$TARGET/asan/$HOST/debug/libdaegun.a" \
         "$RT" -Wl,-rpath,"$(dirname "$RT")" $LINK -o "$CT/rt" >>"$CT/build.log" 2>&1; then
    echo "does not build"; tail -10 "$CT/build.log"; rm -rf "$CT"; exit 1
  elif ! OUT=$("$CT/rt" assets/test-fonts/inter/InterVariable.ttf 2>&1); then
    echo "failed under ASan"; echo "$OUT" | grep -E "FAIL|failed|ERROR|SUMMARY|^ +#[0-9]+ " | head -30; rm -rf "$CT"; exit 1
  else
    echo "clean"
  fi
  rm -rf "$CT"
else
  printf 'lib tests asan ... skipped (nightly absent)\n'
  printf 'c abi, rust asan . skipped (nightly absent)\n'
fi

printf 'c parity ......... '
if ! sh scripts/tools/c-parity.sh; then
  exit 1
fi

printf 'abi parity ....... '
# Only the library this run built: anything else under target/ may predate the header. One line
# unless it fails, when the whole report shows.
abi_out=$(python3 scripts/tools/abi-parity.py "$TARGET/debug/libdaegun.a") || { echo "issues"; echo "$abi_out"; exit 1; }
echo "$abi_out" | tail -1

if rustup run 1.97.1 rustc --version >/dev/null 2>&1; then
  printf 'MSRV 1.97.1 ...... '
  [ "$(rustup run 1.97.1 cargo check --all-features 2>&1 | grep -cE '^error')" -eq 0 ] && echo clean || { echo "issues"; exit 1; }
else
  printf 'MSRV 1.97.1 ...... skipped (toolchain absent)\n'
fi

echo "GATE GREEN ($N tests, $TN with threading, $CN with capi)"
