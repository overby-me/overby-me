"$REF"  info 0 </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" info 0 </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-cli info 0 (Core)"
