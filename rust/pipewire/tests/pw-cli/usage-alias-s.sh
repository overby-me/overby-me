"$REF"  s </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" s </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli s (alias usage)"
