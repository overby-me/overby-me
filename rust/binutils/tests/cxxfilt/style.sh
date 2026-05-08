# Test c++filt -s gnu-v3 — accept the style flag (Itanium ABI)
echo "_Z3fooP1A" > "$TMPDIR/in"
$REF -s gnu-v3 < "$TMPDIR/in" > "$TMPDIR/expected" 2>&1 || true
$RUST -s gnu-v3 < "$TMPDIR/in" > "$TMPDIR/actual" 2>&1 || true
compare "c++filt -s gnu-v3"
