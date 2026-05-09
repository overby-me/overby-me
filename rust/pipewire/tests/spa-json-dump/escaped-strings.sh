cat > "$TMPDIR/in.json" <<'INNER'
quote = "with \"quote\""
backslash = "a\\b"
newline = "line1\nline2"
tab = "a\tb"
INNER
"$REF"  "$TMPDIR/in.json" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.json" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/escaped-strings"
