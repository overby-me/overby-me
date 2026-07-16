"$REF"  -Mump /tmp/nonexistent </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -Mump /tmp/nonexistent </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump -Mump <file> (attached value)"
