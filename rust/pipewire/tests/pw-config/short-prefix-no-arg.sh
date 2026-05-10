"$REF"  -p </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -p </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-config -p (short form requires arg)"
