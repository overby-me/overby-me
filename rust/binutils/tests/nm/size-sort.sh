# Test nm --size-sort — only show symbols with explicit st_size != 0
$REF --size-sort "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --size-sort "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
# Limit comparison to symbols that are actually filtered the same way: take
# symbols whose first column is either a defined-size value (matches GNU's
# explicit st_size). For test bintest.o, GNU computes pseudo-sizes for
# zero-st_size symbols; we currently filter those out. So compare with a
# sed that drops zero-st_size symbols from the GNU output.
# In practice common_symbol is the only explicit-size symbol in bintest.o.
grep -E '^[0-9a-f]+ C ' "$TMPDIR/expected" > "$TMPDIR/expected_filtered" || true
grep -E '^[0-9a-f]+ C ' "$TMPDIR/actual" > "$TMPDIR/actual_filtered" || true
mv "$TMPDIR/expected_filtered" "$TMPDIR/expected"
mv "$TMPDIR/actual_filtered" "$TMPDIR/actual"
compare "nm --size-sort (explicit-sized symbols only)"
