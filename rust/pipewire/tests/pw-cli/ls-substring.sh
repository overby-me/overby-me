# `pw-cli ls Pi` matches every global because the substring "Pi" appears
# in every "PipeWire:Interface:..." type. This exercises the C tool's
# `strstr(g->type, pattern)` substring filter, which we mirror.
"$REF"  ls Pi </dev/null > "$TMPDIR/c.full"  2>"$TMPDIR/c.err"
"$RUST" ls Pi </dev/null > "$TMPDIR/r.full"  2>"$TMPDIR/r.err"

# Strip the connecting client (varies per binary).
strip_client() {
  awk '/type PipeWire:Interface:Client\//{exit} {print}' "$1" > "$2"
}
strip_client "$TMPDIR/c.full"  "$TMPDIR/expected"
strip_client "$TMPDIR/r.full"  "$TMPDIR/actual"
compare "pw-cli ls Pi (substring matches everything)"
