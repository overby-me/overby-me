"$REF"  ls Core </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" ls Core </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli ls Core"
