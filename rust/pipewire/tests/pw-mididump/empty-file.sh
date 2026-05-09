echo > "$TMPDIR/empty.mid"
"$REF"  "$TMPDIR/empty.mid" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" "$TMPDIR/empty.mid" </dev/null > "$TMPDIR/actual"   2>&1 || true
# Normalize the temp file path
sed -i "s#$TMPDIR/empty.mid#TMPFILE#g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump empty.mid (Invalid argument)"
