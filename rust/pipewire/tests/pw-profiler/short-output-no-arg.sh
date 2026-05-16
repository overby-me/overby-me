# pw-profiler -o (short form of --output) requires an argument.
"$REF"  -o </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -o </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-profiler -o (short form requires arg)"
