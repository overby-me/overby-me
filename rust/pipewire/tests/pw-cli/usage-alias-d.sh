"$REF"  d </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" d </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli d (alias usage)"
