"$REF" -c 0 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -c 0 </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "spa-resample/with-c-flag"
