# pw-cli ls with no filter on rich daemon. Strip the connecting Client.
"$REF"  ls </dev/null > "$TMPDIR/c.full"  2>&1
"$RUST" ls </dev/null > "$TMPDIR/r.full"  2>&1
strip_client() {
  awk '/type PipeWire:Interface:Client\//{exit} {print}' "$1" > "$2"
}
strip_client "$TMPDIR/c.full" "$TMPDIR/expected"
strip_client "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-cli ls (rich daemon, sans Client)"
