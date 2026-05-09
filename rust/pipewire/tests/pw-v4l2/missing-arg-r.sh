"$REF"  -r </dev/null > "$TMPDIR/c.full" 2>&1 || true
"$RUST" -r </dev/null > "$TMPDIR/r.full" 2>&1 || true
# Normalize C's full-path argv0 to TOOL.
sed -E "s#/[^[:space:]:]+/pw-v4l2#TOOL#g" "$TMPDIR/c.full" > "$TMPDIR/expected"
sed -E "s#/[^[:space:]:]+/pw-v4l2#TOOL#g" "$TMPDIR/r.full" > "$TMPDIR/actual"
sed -i "s#^pw-v4l2:#TOOL:#" "$TMPDIR/actual"
sed -i "s#^pw-v4l2 -#TOOL -#" "$TMPDIR/actual"
compare "pw-v4l2 -r (missing arg)"
