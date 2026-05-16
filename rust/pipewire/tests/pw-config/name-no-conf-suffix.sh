# pw-config --name=foo (no .conf suffix). C's pw_conf_load_conf_for_context
# (conf.c:1210) rejects names that don't end in .conf with
# "config.name '<name>' does not end with .conf" + Invalid argument
# error + exit code 234 (-EINVAL truncated to u8).
"$REF"  --name=foo > "$TMPDIR/c.full" 2>&1 || true
e_ref=$?
"$RUST" --name=foo > "$TMPDIR/r.full" 2>&1 || true
e_rust=$?
# Normalize timestamps.
sed -E 's|\[[0-9:.]+\]|[TIME]|' "$TMPDIR/c.full" > "$TMPDIR/expected"
sed -E 's|\[[0-9:.]+\]|[TIME]|' "$TMPDIR/r.full" > "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-config --name=foo (rejected: missing .conf suffix → exit 234)"
