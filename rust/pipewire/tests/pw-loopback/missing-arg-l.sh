# pw-loopback -l (latency) requires an argument; getopt errors.
"$REF"  -l </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -l </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-loopback -l (missing latency argument)"
