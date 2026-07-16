# pw-loopback's help text bakes in the current PID for the default node
# name. Normalize PIDs to a sentinel before diffing.
"$REF" --help > "$TMPDIR/c.full" 2>&1
"$RUST" --help > "$TMPDIR/r.full" 2>&1
norm() {
  sed -E "s/pw-loopback-[0-9]+/pw-loopback-PID/g" "$1" > "$2"
}
norm "$TMPDIR/c.full" "$TMPDIR/expected"
norm "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-loopback/help (PID-normalized)"
