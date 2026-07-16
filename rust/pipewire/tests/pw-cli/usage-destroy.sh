# pw-cli destroy with no args prints `Error: "destroy <usage>"` to stderr.
"$REF"  destroy </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" destroy </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli destroy (usage error)"
