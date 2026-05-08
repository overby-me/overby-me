# Test c++filt -t / --types — also demangle type encodings
echo "Pi" > "$TMPDIR/in"
$REF -t < "$TMPDIR/in" > "$TMPDIR/expected" 2>&1 || true
$RUST -t < "$TMPDIR/in" > "$TMPDIR/actual" 2>&1 || true
compare "c++filt -t"
