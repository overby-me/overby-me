"$REF"  -ffoo - - > "$TMPDIR/expected" 2>&1 || true
"$RUST" -ffoo - - > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -ffoo (invalid format → 'bad format foo')"
