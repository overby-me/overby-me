# Test readelf -V / --version-info — display version sections (or
# "No version information" message)
$REF -V "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -V "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -V"
