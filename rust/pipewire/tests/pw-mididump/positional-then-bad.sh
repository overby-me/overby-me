"$REF"  - --bad </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" - --bad </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump - --bad (option-after-positional)"
