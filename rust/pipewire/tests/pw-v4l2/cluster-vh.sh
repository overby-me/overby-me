"$REF"  -vh </dev/null > "$TMPDIR/c.full" 2>&1 || true
"$RUST" -vh </dev/null > "$TMPDIR/r.full" 2>&1 || true
sed -E "s#/[^[:space:]:]+/pw-v4l2#TOOL#g" "$TMPDIR/c.full" > "$TMPDIR/expected"
sed -E "s#/[^[:space:]:]+/pw-v4l2#TOOL#g" "$TMPDIR/r.full" > "$TMPDIR/actual"
sed -i "s#^pw-v4l2 -#TOOL -#" "$TMPDIR/actual"
compare "pw-v4l2 -vh (cluster)"
