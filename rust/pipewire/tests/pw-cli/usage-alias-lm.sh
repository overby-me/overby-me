"$REF"  lm </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" lm </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli lm (alias usage)"
