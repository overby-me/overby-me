# pw-reserve -n (short form of --name) requires an argument.
"$REF"  -n > "$TMPDIR/expected" 2>&1 || true
"$RUST" -n > "$TMPDIR/actual"   2>&1 || true
compare "pw-reserve -n (short form requires arg)"
