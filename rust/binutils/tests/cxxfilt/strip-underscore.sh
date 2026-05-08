# Test c++filt --strip-underscore — strip leading underscore before demangling, restore on failure
cat > "$TMPDIR/input" << 'EOF'
_Z1av
__Z1av
_main
main
EOF
$REF --strip-underscore < "$TMPDIR/input" > "$TMPDIR/expected" 2>&1 || true
$RUST --strip-underscore < "$TMPDIR/input" > "$TMPDIR/actual" 2>&1 || true
compare "c++filt --strip-underscore"
