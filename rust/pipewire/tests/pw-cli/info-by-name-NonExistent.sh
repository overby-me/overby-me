"$REF"  info NonExistent </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info NonExistent </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info NonExistent (by-name lookup miss)"
