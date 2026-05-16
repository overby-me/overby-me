# C: ret = atoi("-3") = -3; ret <= 0 → 'error: bad channels -3'.
"$REF"  --channels=-3 > "$TMPDIR/expected" 2>&1 || true
"$RUST" --channels=-3 > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --channels=-3 (negative → 'bad channels -3')"
