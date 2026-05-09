"$REF"  um </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" um </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli um (alias usage)"
