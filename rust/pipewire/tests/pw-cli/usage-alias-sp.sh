"$REF"  sp </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" sp </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli sp (alias usage)"
