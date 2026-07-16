"$REF" -hV </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -hV </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-profiler/cluster-hV"
