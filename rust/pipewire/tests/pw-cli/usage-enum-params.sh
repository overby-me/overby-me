# pw-cli enum-params with no args prints `Error: "enum-params <usage>"` to stderr.
"$REF"  enum-params </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" enum-params </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli enum-params (usage error)"
