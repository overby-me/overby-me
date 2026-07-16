"$REF"  export-node 1 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" export-node 1 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli export-node 1 (no daemon)"
