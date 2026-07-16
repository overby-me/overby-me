"$REF" /foo.so /bar.so </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" /foo.so /bar.so </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-inspect/two-args"
