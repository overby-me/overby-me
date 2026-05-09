# pw-cli permissions with no args prints `Error: "permissions <usage>"` to stderr.
"$REF"  permissions </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" permissions </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli permissions (usage error)"
