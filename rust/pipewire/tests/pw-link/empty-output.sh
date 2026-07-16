# pw-link -o in the basic daemon: no nodes/ports → no output.
"$REF"  -o </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" -o </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-link -o (basic daemon, empty)"
