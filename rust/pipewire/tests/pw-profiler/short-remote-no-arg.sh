# pw-profiler -r (short form of --remote) requires an argument.
"$REF"  -r </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -r </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-profiler -r (short form requires arg)"
