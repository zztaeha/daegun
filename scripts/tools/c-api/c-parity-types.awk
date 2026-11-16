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
    if (t ~ /^[A-Z][A-Za-z0-9]*$/) print t
  }
}
