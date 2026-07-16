# pw-reserve -a (short form of --appname) requires an argument.
"$REF"  -a > "$TMPDIR/expected" 2>&1 || true
"$RUST" -a > "$TMPDIR/actual"   2>&1 || true
compare "pw-reserve -a (short form requires arg)"
