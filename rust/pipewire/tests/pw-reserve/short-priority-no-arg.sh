# pw-reserve -p (short form of --priority) requires an argument.
"$REF"  -p > "$TMPDIR/expected" 2>&1 || true
"$RUST" -p > "$TMPDIR/actual"   2>&1 || true
compare "pw-reserve -p (short form requires arg)"
