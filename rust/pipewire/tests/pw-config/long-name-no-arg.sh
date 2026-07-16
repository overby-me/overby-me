"$REF"  --name </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --name </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-config --name (long form requires arg)"
