"$REF"  en </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" en </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli en (alias usage)"
