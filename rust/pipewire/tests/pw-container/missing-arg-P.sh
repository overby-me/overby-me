# pw-container -P requires an argument (properties string).
"$REF"  -P </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -P </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-container -P (missing properties argument)"
