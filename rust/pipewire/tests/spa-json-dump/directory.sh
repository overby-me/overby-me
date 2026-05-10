"$REF"  /etc > "$TMPDIR/expected" 2>&1 || true
"$RUST" /etc > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump /etc (directory → mmap ENODEV)"
