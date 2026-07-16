"$REF"  get-permissions 0 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" get-permissions 0 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli get-permissions 0 (no daemon)"
