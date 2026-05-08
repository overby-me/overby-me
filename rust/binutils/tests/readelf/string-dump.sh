# Test readelf -p .shstrtab — string dump of section header string table
$REF -p .shstrtab "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -p .shstrtab "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -p .shstrtab"
