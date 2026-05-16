# Test readelf -p .data — should print "No strings found in this section." for sections of all-non-printable bytes
$REF -p .data "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -p .data "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -p .data (no strings)"
