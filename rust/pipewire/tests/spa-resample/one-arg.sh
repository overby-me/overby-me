"$REF" foo.wav </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" foo.wav </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "spa-resample/one-arg"
