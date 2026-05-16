# Test c++filt -n / --no-strip-underscore — don't strip leading underscores
echo "_Z1av" > "$TMPDIR/in"
$REF -n < "$TMPDIR/in" > "$TMPDIR/expected" 2>&1 || true
$RUST -n < "$TMPDIR/in" > "$TMPDIR/actual" 2>&1 || true
compare "c++filt -n"
