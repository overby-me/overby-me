# `pw-cli '#comment'` is treated by C's parse() as a comment line:
# strchr('#') truncates the buffer to "", then the empty-input no-op
# branch fires and the command runs silently with exit 0.
"$REF"  "#comment" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "#comment" </dev/null > "$TMPDIR/actual"   2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-cli '#comment' (entire input is a comment, silent no-op)"
