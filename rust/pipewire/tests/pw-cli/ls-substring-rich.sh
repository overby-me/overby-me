# Rich daemon: `ls Pi` matches every global. Tests substring filter
# behavior with the larger registry that includes a Node.
"$REF"  ls Pi </dev/null > "$TMPDIR/c.full"  2>"$TMPDIR/c.err"
"$RUST" ls Pi </dev/null > "$TMPDIR/r.full"  2>"$TMPDIR/r.err"

strip_client() {
  awk '/type PipeWire:Interface:Client\//{exit} {print}' "$1" > "$2"
}
strip_client "$TMPDIR/c.full"  "$TMPDIR/expected"
strip_client "$TMPDIR/r.full"  "$TMPDIR/actual"
compare "pw-cli ls Pi (rich daemon)"
