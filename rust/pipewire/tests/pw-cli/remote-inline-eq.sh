"$REF"  --remote=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --remote=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli --remote=foo (REPL connect-attempt)"
