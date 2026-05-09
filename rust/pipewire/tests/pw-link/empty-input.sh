# pw-link -i in the basic daemon: no nodes/ports → no output.
"$REF"  -i </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" -i </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-link -i (basic daemon, empty)"
