# C: ret = atoi("0") = 0; ret <= 0 → 'error: bad channels 0'.
"$REF"  --channels=0 > "$TMPDIR/expected" 2>&1 || true
"$RUST" --channels=0 > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --channels=0 (ret <= 0 → 'bad channels 0')"
