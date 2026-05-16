# Test size -t (show totals) with multiple files
cp "$TESTOBJ" "$TMPDIR/copy.o"
$REF -t "$TESTOBJ" "$TMPDIR/copy.o" > "$TMPDIR/expected" 2>&1 || true
$RUST -t "$TESTOBJ" "$TMPDIR/copy.o" > "$TMPDIR/actual" 2>&1 || true
compare "size -t (totals)"