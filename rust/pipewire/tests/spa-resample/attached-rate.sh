"$REF"  -r44100 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -r44100 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -r44100 (attached short value parses)"
