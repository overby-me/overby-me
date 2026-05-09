"$REF"  ls Metadata </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" ls Metadata </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli ls Metadata (rich daemon)"
