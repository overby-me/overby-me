"$REF"  -I </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -I </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link -I (without -i/-o/-l)"
