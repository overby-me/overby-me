# pw-profiler -n (short form of --iterations) requires an argument.
"$REF"  -n </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -n </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-profiler -n (short form requires arg)"
