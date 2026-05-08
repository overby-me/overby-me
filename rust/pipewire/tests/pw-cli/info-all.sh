# `info all` walks every registry global. Apply the same Client-block strip
# as ls-all-normalized so the connecting client (which differs per binary)
# doesn't pollute the diff.
"$REF"  info all </dev/null > "$TMPDIR/c.full" 2>"$TMPDIR/c.err"
"$RUST" info all </dev/null > "$TMPDIR/r.full" 2>"$TMPDIR/r.err"

strip_client() {
  awk '
    /type: PipeWire:Interface:Client\//{ skip=1 }
    skip { next }
    { print }
  ' "$1" > "$2"
}
strip_client "$TMPDIR/c.full" "$TMPDIR/expected"
strip_client "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-cli info all (sans Client)"
