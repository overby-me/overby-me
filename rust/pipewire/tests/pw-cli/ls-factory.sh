"$REF"  ls Factory </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" ls Factory </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli ls Factory"
