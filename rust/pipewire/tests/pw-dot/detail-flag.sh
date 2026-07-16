# C: case 'd' prints "detail option enabled" to stderr.
"$REF"  --detail </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --detail </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dot --detail (prints 'detail option enabled' to stderr)"
