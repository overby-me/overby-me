# pw-loopback -n (node name) requires an argument; getopt errors.
"$REF"  -n </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -n </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-loopback -n (missing node-name argument)"
