# Rich daemon: pw-link -i -I shows port ids.
"$REF"  -i -I </dev/null > "$TMPDIR/expected" 2>"$TMPDIR/expected.err"
"$RUST" -i -I </dev/null > "$TMPDIR/actual"   2>"$TMPDIR/actual.err"
compare "pw-link -i -I (rich daemon)"
