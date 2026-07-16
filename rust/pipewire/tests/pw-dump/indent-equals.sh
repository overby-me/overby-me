PIPEWIRE_REMOTE=non-existent "$REF"  --indent=4 </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
PIPEWIRE_REMOTE=non-existent "$RUST" --indent=4 </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-dump --indent=4 (inline form accepted)"
