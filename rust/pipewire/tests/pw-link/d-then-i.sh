"$REF"  -d -i </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -d -i </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link -d -i (last mode flag wins)"
