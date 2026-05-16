# pw-loopback -c (channels) requires an argument; getopt errors.
"$REF"  -c </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -c </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-loopback -c (missing channels argument)"
