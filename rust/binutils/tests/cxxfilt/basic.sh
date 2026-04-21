# Test c++filt demangling of C++ symbols
echo "_Z3fooi" | $REF > "$TMPDIR/expected" 2>&1 || true
echo "_Z3fooi" | $RUST > "$TMPDIR/actual" 2>&1 || true
compare "c++filt basic demangling"