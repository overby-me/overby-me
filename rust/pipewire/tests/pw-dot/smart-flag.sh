# C: case 's' prints "smart option enabled" to stderr.
"$REF"  --smart </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --smart </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dot --smart (prints 'smart option enabled' to stderr)"
