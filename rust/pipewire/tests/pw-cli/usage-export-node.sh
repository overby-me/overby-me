"$REF"  export-node </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" export-node </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli export-node (usage error)"
