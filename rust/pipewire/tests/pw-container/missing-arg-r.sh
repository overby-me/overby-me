# pw-container -r requires an argument (remote daemon name).
"$REF"  -r </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -r </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-container -r (missing remote argument)"
