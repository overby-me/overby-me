"$REF" --version=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --version=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-top/version-with-arg"
