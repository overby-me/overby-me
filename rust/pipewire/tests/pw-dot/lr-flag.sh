# C: case 'L' prints "set rank direction to LR" to stderr.
"$REF"  --lr </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --lr </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dot --lr (prints 'set rank direction to LR' to stderr)"
