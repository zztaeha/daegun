#!/bin/sh
set -eu
cd "$(dirname "$0")/../../.."

# The text calls, not shape.rs beside them: its pub items are the shaper's glue, reached through Font.
TEXT=$(ls src/daecore/src/daekit/text/*.rs | grep -v '/shape\.rs$')
FILES="src/api/rust/*.rs $TEXT"
REEXPORTS="src/lib.rs src/api/rust/*.rs $TEXT"

modules=" bytes class format gradient shape state metrics layout unicode outline color math tables raw raster "

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
types=$(awk -f scripts/tools/c-api/c-parity-types.awk $REEXPORTS)
types=$(printf '%s\nRamp\n' "$types" | sort -u)

aliases=$(sed -n 's/^pub use .*::\([A-Za-z_][A-Za-z_0-9]*\) as \([A-Za-z_][A-Za-z_0-9]*\);.*/\2 \1/p' \
            $REEXPORTS 2>/dev/null)

for ty in $types; do
  # A type re-exported under another name has no `impl <alias>` to find, so resolve it first or its
  # methods are skipped in silence.
  real=$(echo "$aliases" | awk -v a="$ty" '$1 == a { print $2; exit }')
  [ -n "$real" ] && ty="$real"
  files=$(grep -rl "^impl.*[[:space:]]${ty}[[:space:]<{]" src/daecore/src src/api/rust 2>/dev/null | head -20)
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
debug=$(cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')/debug
lib=$(ls -t "$debug/libdaegun.dylib" "$debug/libdaegun.so" "$debug/libdaegun.a" 2>/dev/null | head -1)
case "$lib" in
  *.dylib) syms=$(nm -gU "$lib" 2>/dev/null || true) ;;
  *.so)    syms=$(nm -D --defined-only "$lib" 2>/dev/null || true) ;;
  *.a)     syms=$(nm -g "$lib" 2>/dev/null || true) ;;
  *)       echo "not built in $debug (run: cargo rustc --features capi --crate-type staticlib)"; exit 1 ;;
esac
c=$(echo "$syms" | grep -oE '_?daegun_[a-z_0-9]+' | sed -E 's/^_?daegun_//' | sort -u)

# Under C's own names: an options builder is a field of C's struct, an `Error`'s kind its status, a paint's children the
# nodes up to its `end`, a path's view and segments its verbs and points, a kept-list call its plain twin, a count its
# list's call asked for the count alone, and a visit the list that lends its items.
renamed=" children path paths view with_features with_language from_bytes from_collection from_static with_transform\
 with_hinting with_stroke with_embolden with_oblique with_cap with_join with_language_system math_constants\
 named_instances build_font\
 parse_delta_set_index_map line_height parts\
 grayscale horizontal vertical unfiltered\
 from_vec flatten_into quads_into segments finish blend from_colr lower resolve parse with_interpolation\
 gradient shapes to_path root node\
 units_per_em glyph_count pdf_flags bits os2 stat palettes base_script\
 normalized hinted_outline color_scene paint_graph set_cache_budget clear_cache set_index_allowance\
 kind message with_palette with_foreground with_tolerance\
 with_align with_base_direction with_line_height with_max_inline_size with_max_lines with_segmenter\
 with_strategy with_tab_size with_text_orientation with_writing_mode\
 with_beginning_of_text with_cluster_level with_context with_direction with_ignorables with_invisible_glyph\
 with_not_found_glyph with_not_found_variation_selector_glyph with_point_size with_report_tatweel_positions\
 with_report_unsafe_to_concat with_script with_seed_script with_suppress_dotted_circle\
 with_darken with_gamma with_layout with_offset with_outline\
 release_thread reusing palette_count visit_names "

# Rust's alone: C writes an Rgba, tests a flag and does 2x3 matrix math by hand, reads scenes and paint graphs rather
# than building them, packs a tag with DAEGUN_TAG, names a script by its ISO 15924 code itself, keeps an image it was
# lent by copying its pixels and gives a path room by clearing one it keeps.
unreachable=" fade opaque contains push push_path push_gradient push_shapes with invert concat\
 to_bytes iso15924 from_iso15924 to_bitmap image with_capacity "

total=$(echo "$rust" | grep -c . || true)
missing=""
loose=""
covered=0
alone=0
for name in $rust; do
  if echo "$unreachable" | grep -q " ${name} "; then
    alone=$((alone + 1))
  elif echo "$renamed" | grep -q " ${name} "; then
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

total=$((total - alone))
pct=$((covered * 100 / (total > 0 ? total : 1)))
printf '%d of %d reachable from C (%d%%), %d Rust'"'"'s alone by design\n' "$covered" "$total" "$pct" "$alone"

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
