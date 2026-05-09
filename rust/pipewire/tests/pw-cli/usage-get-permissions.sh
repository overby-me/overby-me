"$REF"  get-permissions </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" get-permissions </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli get-permissions (usage error)"
