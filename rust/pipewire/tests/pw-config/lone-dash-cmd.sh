"$REF"  - </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" - </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-config - (lone dash positional, exit 0)"
