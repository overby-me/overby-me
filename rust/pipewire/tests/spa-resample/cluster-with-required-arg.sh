"$REF"  -vc 0 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -vc 0 - - </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-resample -vc 0 (cluster with required-arg flag last)"
