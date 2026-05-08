# Test c++filt -i — don't add a leading underscore
echo "_Z1av" > "$TMPDIR/in"
$REF -i < "$TMPDIR/in" > "$TMPDIR/expected" 2>&1 || true
$RUST -i < "$TMPDIR/in" > "$TMPDIR/actual" 2>&1 || true
compare "c++filt -i"
