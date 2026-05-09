# Look up first global of type `Metadata` by substring match.
"$REF"  info Metadata </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info Metadata </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info Metadata (by-name)"
