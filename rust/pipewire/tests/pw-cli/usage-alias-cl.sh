"$REF"  cl </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" cl </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli cl (alias usage)"
