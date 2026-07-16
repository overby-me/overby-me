"$REF"  --prefix </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --prefix </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-config --prefix (long form requires arg)"
