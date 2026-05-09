"$REF"  -Vxx </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -Vxx </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump/cluster-Vxx"
