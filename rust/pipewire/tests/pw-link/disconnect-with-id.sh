"$REF"  -d 1 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -d 1 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link -d 1 (disconnect by id)"
