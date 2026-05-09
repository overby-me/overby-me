# pw-cli create-node with no args prints `Error: "create-node <usage>"` to stderr.
"$REF"  create-node </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" create-node </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli create-node (usage error)"
