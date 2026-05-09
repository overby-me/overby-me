"$REF"  e </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" e </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli e (alias usage)"
