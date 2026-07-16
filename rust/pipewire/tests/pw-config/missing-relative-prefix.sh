# pw-config -p <relative-path> -n <name>. C's get_config_path tries
# get_abs_path first; for relative prefixes it falls through env/home/
# configdir lookups WITHOUT calling conf_load directly. So the 425 W
# line from conf_load() does NOT appear here (unlike the absolute-path
# case covered by missing-prefix-path.sh).
"$REF"  --prefix=relpath --name=test.conf > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --prefix=relpath --name=test.conf > "$TMPDIR/actual"   2>&1 || true
e_rust=$?
sed -i -E 's|\[[0-9:.]+\]|[TIME]|g; s|0x[0-9a-f]+|0xPTR|g' "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-config -p relpath (no 425 W line; relative prefix bypasses direct fopen)"
