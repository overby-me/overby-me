"$REF"  --config </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --config </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pipewire --config (long config requires arg, exit 234)"
