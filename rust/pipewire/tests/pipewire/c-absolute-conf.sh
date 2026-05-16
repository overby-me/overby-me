# pipewire -c <absolute-path> — C's pw_conf_load_conf takes the
# get_abs_path branch, calls conf_load() directly (which emits the
# 425 W line on open failure) BEFORE the 1182 try_load_conf log line.
# Relative paths skip conf_load() and don't emit 425.
"$REF"  -c /nonexistent.conf </dev/null > "$TMPDIR/c.full" 2>&1 || true
e_ref=$?
"$RUST" -c /nonexistent.conf </dev/null > "$TMPDIR/r.full" 2>&1 || true
e_rust=$?
sed -E 's|\[[0-9:.]+\]|[TIME]|; s|0x[0-9a-f]+|0xPTR|g' "$TMPDIR/c.full" > "$TMPDIR/expected"
sed -E 's|\[[0-9:.]+\]|[TIME]|; s|0x[0-9a-f]+|0xPTR|g' "$TMPDIR/r.full" > "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pipewire -c /nonexistent.conf (absolute → emits 425 W line)"
