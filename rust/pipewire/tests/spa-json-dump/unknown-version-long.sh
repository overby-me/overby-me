# spa-json-dump doesn't declare --version. Long-form rejected.
"$REF"  --version </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --version </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump --version (unrecognized: not declared)"
