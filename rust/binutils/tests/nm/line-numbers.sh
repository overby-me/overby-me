# Test nm -l / --line-numbers — annotate symbols with file:line (no-op
# for objects without DWARF)
$REF -l "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -l "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -l"
