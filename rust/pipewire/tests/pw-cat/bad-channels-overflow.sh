# C: ret = atoi("999999999999"); i32 wrap-around. Whatever the wrapped
# value, the error format is 'error: bad channels <wrapped>'.
"$REF"  --channels=999999999999 > "$TMPDIR/expected" 2>&1 || true
"$RUST" --channels=999999999999 > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --channels=999999999999 (atoi wraps to negative i32)"
