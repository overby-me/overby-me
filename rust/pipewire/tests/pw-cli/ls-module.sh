"$REF"  ls Module </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" ls Module </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli ls Module"
