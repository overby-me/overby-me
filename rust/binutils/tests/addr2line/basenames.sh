# Test addr2line --basenames — strip directory components
$REF --basenames -e "$TESTOBJ" 0x0 > "$TMPDIR/expected" 2>&1 || true
$RUST --basenames -e "$TESTOBJ" 0x0 > "$TMPDIR/actual" 2>&1 || true
compare "addr2line --basenames"
