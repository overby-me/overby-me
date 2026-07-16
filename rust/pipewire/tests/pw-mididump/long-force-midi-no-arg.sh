# pw-mididump --force-midi (long form of -M) requires an argument.
"$REF"  --force-midi </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --force-midi </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump --force-midi (long form requires arg)"
