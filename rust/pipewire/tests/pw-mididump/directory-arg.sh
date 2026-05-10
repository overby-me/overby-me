"$REF"  /tmp > "$TMPDIR/expected" 2>&1 || true
"$RUST" /tmp > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump /tmp (directory → Invalid argument)"
