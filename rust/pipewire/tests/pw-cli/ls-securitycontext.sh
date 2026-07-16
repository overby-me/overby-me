"$REF"  ls SecurityContext </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" ls SecurityContext </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli ls SecurityContext"
