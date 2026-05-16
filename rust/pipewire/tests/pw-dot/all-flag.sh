# C: case 'a' prints "all option enabled" to stderr from option-parse.
"$REF"  --all </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --all </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dot --all (prints 'all option enabled' to stderr)"
