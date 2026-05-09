"$REF"  gp </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" gp </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli gp (alias usage)"
