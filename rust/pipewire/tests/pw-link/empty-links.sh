# pw-link -l in the basic daemon: no links → no output.
"$REF"  -l </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" -l </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-link -l (basic daemon, empty)"
