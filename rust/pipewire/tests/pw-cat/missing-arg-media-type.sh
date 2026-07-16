"$REF" --media-type </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --media-type </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat/missing-arg-media-type"
