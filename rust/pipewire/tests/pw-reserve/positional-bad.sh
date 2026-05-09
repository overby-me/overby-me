# pw-reserve doesn't take positional args; without `-n NAME` it errors.
"$REF"  foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-reserve foo (positional ignored, valid-name check fails)"
