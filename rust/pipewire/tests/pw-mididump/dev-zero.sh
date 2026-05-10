# /dev/zero reads forever; pw-mididump must bound the read and report
# "Invalid argument" (MThd magic mismatch).
"$REF" /dev/zero </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" /dev/zero </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-mididump /dev/zero (unbounded source → Invalid argument)"
