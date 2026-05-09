"$REF" -R </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -R </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat/missing-arg-R"
