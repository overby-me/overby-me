# `info all` against the rich daemon: every interface gets exercised,
# including the null-audio-sink Node. Strip the connecting client section
# (its props differ per session).
"$REF"  info all </dev/null > "$TMPDIR/c.full"  2>"$TMPDIR/c.err"
"$RUST" info all </dev/null > "$TMPDIR/r.full"  2>"$TMPDIR/r.err"

strip_client() {
  awk '
    /type: PipeWire:Interface:Client\//{ skip=1 }
    skip { next }
    { print }
  ' "$1" > "$2"
}
strip_client "$TMPDIR/c.full" "$TMPDIR/expected"
strip_client "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-cli info all (rich daemon, sans Client)"
