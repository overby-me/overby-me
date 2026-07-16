"$REF"  -P </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -P </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pipewire -P (short properties requires arg, exit 234)"
