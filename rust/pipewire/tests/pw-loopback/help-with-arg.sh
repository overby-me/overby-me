"$REF" --help=foo > "$TMPDIR/c.full" 2>&1 || true
"$RUST" --help=foo > "$TMPDIR/r.full" 2>&1 || true
norm() { sed -E "s/pw-loopback-[0-9]+/pw-loopback-PID/g" "$1" > "$2"; }
norm "$TMPDIR/c.full" "$TMPDIR/expected"
norm "$TMPDIR/r.full" "$TMPDIR/actual"
compare "pw-loopback/help-with-arg"
