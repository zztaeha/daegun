# Sourced by the scripts that record with xctrace. Each recording leaves a kernel trace in the temp directory and
# a scratch directory in the user cache, hundreds of MB between them, and xctrace removes neither.
[ -n "${DEVELOPER_DIR:-}" ] || [ ! -d /Applications/Xcode-beta.app ] \
    || export DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer
xcrun xctrace version >/dev/null 2>&1 || { echo "xctrace not found: install Xcode, or set DEVELOPER_DIR" >&2; exit 1; }
XCTRACE_SINCE=$(mktemp)
trap 'rm -f "$XCTRACE_SINCE"' EXIT

# This run's leftovers, and any over an hour old that a killed run left behind.
xctrace_clean() {
    for dir in "${TMPDIR:-/tmp}" "$(getconf DARWIN_USER_CACHE_DIR)com.apple.dt.InstrumentsCLI/path_manager"; do
        find "$dir" -maxdepth 1 \( -name 'instruments*.ktrace' -o -name 'xrtmp__*' \) \
            \( -newer "$XCTRACE_SINCE" -o -mmin +60 \) -exec rm -rf {} + 2>/dev/null || true
    done
}
