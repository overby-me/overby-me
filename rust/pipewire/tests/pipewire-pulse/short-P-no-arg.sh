# pipewire-pulse -P (short --properties) requires an argument.
"$REF"  -P </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -P </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pipewire-pulse -P (short form requires arg)"
