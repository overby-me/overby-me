"$REF"  -c </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -c </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pipewire -c (short config requires arg, exit 234)"
