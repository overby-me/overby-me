# `pw-cli info <name>` looks up the first global whose type contains
# `<name>`. This mirrors the C tool's `find_global` substring fallback
# when the input isn't numeric.
"$REF"  info Core </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" info Core </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli info Core (by-name lookup)"
