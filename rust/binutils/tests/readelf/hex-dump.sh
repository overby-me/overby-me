# Test readelf --hex-dump=.shstrtab — hex dump of a named section
$REF --hex-dump=.shstrtab "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --hex-dump=.shstrtab "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf --hex-dump"
