"$REF"  --properties </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --properties </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-acp-tool --properties (long form requires arg)"
