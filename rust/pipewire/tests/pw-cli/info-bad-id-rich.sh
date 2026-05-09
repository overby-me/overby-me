"$REF"  info 99999 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info 99999 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info 99999 (rich daemon)"
