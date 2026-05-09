"$REF"  spr </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" spr </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-acp-tool/spr-no-args"
