"$REF" --bad </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --bad </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "spa-resample/bad-flag"
