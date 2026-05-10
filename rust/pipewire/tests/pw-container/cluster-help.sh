"$REF"  -hh </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -hh </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-container -hh (cluster -h short-circuits)"
