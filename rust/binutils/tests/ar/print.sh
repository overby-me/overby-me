# Test ar -p — print contents of archive members to stdout
cp "$TESTOBJ" "$TMPDIR/m.o"
$REF cr "$TMPDIR/ar.a" "$TMPDIR/m.o" 2>&1 || true
$REF -p "$TMPDIR/ar.a" > "$TMPDIR/expected" 2>&1 || true
$RUST -p "$TMPDIR/ar.a" > "$TMPDIR/actual" 2>&1 || true
compare "ar -p (print archive contents)"
