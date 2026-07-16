"$REF"  info 4 </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" info 4 </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli info 4 (Factory, rich daemon)"
