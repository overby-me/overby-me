"$REF"  d 99 garbage </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" d 99 garbage </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli d 99 garbage (alias-aware unknown global)"
