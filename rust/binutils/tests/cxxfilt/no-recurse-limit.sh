# Test c++filt --no-recurse-limit — accept the demangler knob
echo "_Z3fooP1A" > "$TMPDIR/in"
$REF --no-recurse-limit < "$TMPDIR/in" > "$TMPDIR/expected" 2>&1 || true
$RUST --no-recurse-limit < "$TMPDIR/in" > "$TMPDIR/actual" 2>&1 || true
compare "c++filt --no-recurse-limit"
