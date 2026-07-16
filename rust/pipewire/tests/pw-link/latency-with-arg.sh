"$REF"  --latency=100 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --latency=100 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link --latency=100 (no-arg flag rejects value)"
