# `pw-cli 'help # trailing'` — C truncates at '#', leaving "help " to
# be parsed. We run the `help` command and ignore the comment.
"$REF"  "help # trailing comment" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "help # trailing comment" </dev/null > "$TMPDIR/actual"   2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-cli 'help # comment' (truncate at # then run help)"
