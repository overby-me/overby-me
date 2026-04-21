# Test c++filt with multiple mangled symbols
cat > "$TMPDIR/input" << 'EOF'
_Z3fooi
_Z3barv
_ZN3Foo3bazEi
not_mangled
_ZNSt6vectorIiSaIiEE9push_backERKi
EOF
$REF < "$TMPDIR/input" > "$TMPDIR/expected" 2>&1 || true
$RUST < "$TMPDIR/input" > "$TMPDIR/actual" 2>&1 || true
compare "c++filt multiple symbols"