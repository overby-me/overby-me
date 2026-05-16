# Test ar: create an archive and list its contents
# Create a second object
cp "$TESTOBJ" "$TMPDIR/bintest2.o"
$REF cr "$TMPDIR/ref.a" "$TESTOBJ" "$TMPDIR/bintest2.o" 2>&1 || true
$RUST cr "$TMPDIR/rust.a" "$TESTOBJ" "$TMPDIR/bintest2.o" 2>&1 || true

# List contents of both archives using the same tool (reference) to compare archive format
$REF t "$TMPDIR/ref.a" > "$TMPDIR/expected" 2>&1 || true
$REF t "$TMPDIR/rust.a" > "$TMPDIR/actual" 2>&1 || true
compare "ar create and list"