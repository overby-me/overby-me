"$REF"  --remote </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --remote </dev/null > "$TMPDIR/actual"   2>&1 || true
sed -i "s/pw-loopback-[0-9]*/pw-loopback-PID/g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-loopback --remote (long form requires arg)"
