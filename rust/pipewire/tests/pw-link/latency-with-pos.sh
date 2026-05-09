"$REF"  -t foo bar </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -t foo bar </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link -t foo bar (latency mode skips link)"
