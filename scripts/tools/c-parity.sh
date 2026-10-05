#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."

FILES="src/daegun/api/*.rs src/daegun/text/*.rs"
REEXPORTS="src/daegun/lib.rs src/daegun/api/*.rs src/daegun/text/*.rs"

modules=" bytes class colr format gradient matrix paint shape state "

rust=$(awk '
  FILENAME != last { last = FILENAME; delete private; depth = 0; skip = 0 }
  /^pub\(crate\) (struct|enum) [A-Za-z_]/ { split($0, a, " "); gsub(/[<({].*/, "", a[3]); private[a[3]] = 1 }
  /^(struct|enum) [A-Za-z_]/             { split($0, a, " "); gsub(/[<({].*/, "", a[2]); private[a[2]] = 1 }
  /^impl( <[^>]*>)? [A-Za-z_]/ {
    t = $0; sub(/^impl( <[^>]*>)? /, "", t); sub(/ .*/, "", t); gsub(/[<({].*/, "", t)
    skip = (t in private) ? 1 : 0
  }
  /^}/ { skip = 0 }
  skip == 0 && /^[[:space:]]*pub (const )?fn [a-z_0-9]+/ {
    line = $0
    sub(/.*fn /, "", line); sub(/[^a-z_0-9].*/, "", line)
    print line
  }
' $FILES | sort -u)

reexported=$(awk '
  /^[[:space:]]*pub use / { inuse = 1; buf = ""; depth = 0 }
  inuse {
    stripped = $0
    sub(/\/\/.*/, "", stripped)
    buf = buf " " stripped
    n = length(stripped)
    for (i = 1; i <= n; i++) {
      ch = substr(stripped, i, 1)
      if (ch == "{") depth++
      else if (ch == "}") depth--
      else if (ch == ";" && depth == 0) { inuse = 0; emit(buf); buf = ""; break }
    }
  }
  function emit(line,   parts, n, i, t) {
    if (line ~ /\{/) { sub(/^[^{]*\{/, "", line); sub(/\}[^}]*$/, "", line) }
    else { sub(/^[[:space:]]*pub use[[:space:]]*/, "", line); sub(/;.*$/, "", line) }
    n = split(line, parts, ",")
    for (i = 1; i <= n; i++) {
      t = parts[i]
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", t)
      if (t ~ /[[:space:]]as[[:space:]]/) sub(/^.*[[:space:]]as[[:space:]]+/, "", t)
      sub(/^.*::/, "", t)
      if (t ~ /^[a-z_][a-z_0-9]*$/) print t
    }
  }
' $REEXPORTS | sort -u)

for name in $reexported; do
  echo "$modules" | grep -q " ${name} " || rust="$rust
$name"
done

# `paint::gradient` is re-exported whole, so no `pub use` names its Ramp.
types=$(awk -f scripts/tools/c-parity-types.awk $REEXPORTS)
types=$(printf '%s\nRamp\n' "$types" | sort -u)

aliases=$(sed -n 's/^pub use .*::\([A-Za-z_][A-Za-z_0-9]*\) as \([A-Za-z_][A-Za-z_0-9]*\);.*/\2 \1/p' \
            $REEXPORTS 2>/dev/null)

for ty in $types; do
  # A type re-exported under another name has no `impl <alias>` to find, so resolve it first or its
  # methods are skipped in silence, as DisplayList's would be under `as ColorScene`.
  real=$(echo "$aliases" | awk -v a="$ty" '$1 == a { print $2; exit }')
  [ -n "$real" ] && ty="$real"
  files=$(grep -rl "^impl.*[[:space:]]${ty}[[:space:]<{]" src/daecore/src src/daegun 2>/dev/null | head -20)
  [ -z "$files" ] && continue
  methods=$(awk -v T="$ty" '
    $0 ~ ("^impl(<[^>]*>)?[[:space:]]+" T "([<[:space:]{]|$)") { inimpl = 1; next }
    inimpl && /^}/ { inimpl = 0 }
    inimpl && /^[[:space:]]*pub (const )?fn [a-z_0-9]+/ {
      line = $0; sub(/.*fn /, "", line); sub(/[^a-z_0-9].*/, "", line); print line
    }
  ' $files)
  rust="$rust
$methods"
done
rust=$(echo "$rust" | grep -c . >/dev/null && echo "$rust" | sort -u)

# Where cargo builds, which CARGO_TARGET_DIR or a config can move. No library is a failure: passing on
# nothing is how a check goes quiet.
debug=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')/debug
lib=$(ls -t "$debug/libdaegun.dylib" "$debug/libdaegun.so" "$debug/libdaegun.a" 2>/dev/null | head -1)
case "$lib" in
  *.dylib) syms=$(nm -gU "$lib" 2>/dev/null || true) ;;
  *.so)    syms=$(nm -D --defined-only "$lib" 2>/dev/null || true) ;;
  *.a)     syms=$(nm -g "$lib" 2>/dev/null || true) ;;
  *)       echo "not built in $debug (run: cargo rustc --features capi --crate-type staticlib)"; exit 1 ;;
esac
c=$(echo "$syms" | grep -oE '_?daegun_[a-z_0-9]+' | sed -E 's/^_?daegun_//' | sort -u)

renamed=" from_bytes from_ttc with_transform with_hinting with_stroke\
 with_embolden with_oblique math_constants named_instances build_font\
 parse_item_variation_store parse_delta_set_index_map precompute_region_scalars\
 compute_ivs_delta_f64 draw_hinted line_height parts\
 grayscale horizontal vertical unfiltered\
 from_vec finish blend from_colr lower resolve_stops parse with_interpolation with_axis_count is_hidden "

# Rust's alone: C writes an Rgba as four bytes (`fade`, `opaque`), reads scenes rather than building
# them (`push`, `push_path`), and does a 2x3 matrix's math by hand (`invert`, `concat`).
unreachable=" fade opaque push push_path invert concat "

total=$(echo "$rust" | grep -c . || true)
missing=""
loose=""
covered=0
for name in $rust; do
  if echo "$renamed" | grep -q " ${name} " || echo "$unreachable" | grep -q " ${name} "; then
    covered=$((covered + 1))
  elif echo "$c" | grep -qE "(^|_)${name}$"; then
    covered=$((covered + 1))
  elif echo "$c" | grep -qE "(^|_)${name}_"; then
    covered=$((covered + 1))
    loose="$loose $name"
  else
    missing="$missing $name"
  fi
done

pct=$((covered * 100 / (total > 0 ? total : 1)))
printf '%d of %d reachable from C (%d%%)\n' "$covered" "$total" "$pct"

if [ -n "$loose" ]; then
  printf '  (loosely matched, check by hand:%s)\n' "$loose"
fi

if [ "${1:-}" = "--list" ] && [ -n "$missing" ]; then
  echo "$missing" | tr ' ' '\n' | grep -v '^$' | sed 's/^/    /'
fi

if [ -n "$missing" ] || [ -n "$loose" ]; then
  [ -n "$missing" ] && printf 'not reachable from C:%s\n' "$missing"
  [ -n "$loose" ] && printf 'matched only loosely, so not confirmed:%s\n' "$loose"
  exit 1
fi
