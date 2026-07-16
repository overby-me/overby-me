"$REF"  cn </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" cn </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-cli cn (alias usage)"
