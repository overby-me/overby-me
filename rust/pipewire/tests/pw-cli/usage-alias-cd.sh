"$REF"  cd </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" cd </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli cd (alias usage)"
