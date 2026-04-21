# Test ar: create archive then extract a member
cp "$TESTOBJ" "$TMPDIR/first.o"
cp "$TESTOBJ" "$TMPDIR/second.o"
$RUST cr "$TMPDIR/test.a" "$TMPDIR/first.o" "$TMPDIR/second.o" 2>&1 || true

# Extract using both tools and compare the extracted file
mkdir -p "$TMPDIR/ref_extract" "$TMPDIR/rust_extract"
cd "$TMPDIR/ref_extract" && $REF x "$TMPDIR/test.a" 2>&1 || true
cd "$TMPDIR/rust_extract" && $RUST x "$TMPDIR/test.a" 2>&1 || true

# Compare extracted files
diff "$TMPDIR/ref_extract/first.o" "$TMPDIR/rust_extract/first.o" && \
diff "$TMPDIR/ref_extract/second.o" "$TMPDIR/rust_extract/second.o" && \
echo "PASS: ar extract" || { echo "FAIL: ar extract"; exit 1; }