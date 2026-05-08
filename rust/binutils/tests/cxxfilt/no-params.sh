# Test c++filt -p / --no-params — drop the function-arguments part
echo "_Z3fooPi" > "$TMPDIR/in"
$REF -p < "$TMPDIR/in" > "$TMPDIR/expected" 2>&1 || true
$RUST -p < "$TMPDIR/in" > "$TMPDIR/actual" 2>&1 || true
compare "c++filt -p"
