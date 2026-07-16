"$REF"  -i -d </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -i -d </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link -i -d (last mode flag wins)"
