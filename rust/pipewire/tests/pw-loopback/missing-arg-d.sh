# pw-loopback -d (delay) requires an argument; getopt errors.
"$REF"  -d </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -d </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-loopback -d (missing delay argument)"
