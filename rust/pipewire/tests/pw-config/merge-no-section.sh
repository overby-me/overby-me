"$REF" merge </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" merge </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-config/merge-no-section"
