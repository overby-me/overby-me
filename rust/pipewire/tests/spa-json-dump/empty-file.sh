"$REF"  /dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" /dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump /dev/null (empty file → mmap EINVAL)"
