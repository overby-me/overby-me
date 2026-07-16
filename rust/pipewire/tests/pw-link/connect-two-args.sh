"$REF"  port-a port-b </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" port-a port-b </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link port-a port-b (connect)"
