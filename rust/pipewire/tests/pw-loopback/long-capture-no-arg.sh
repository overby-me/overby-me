"$REF"  --capture </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --capture </dev/null > "$TMPDIR/actual"   2>&1 || true
sed -i "s/pw-loopback-[0-9]*/pw-loopback-PID/g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-loopback --capture (long form requires arg)"
