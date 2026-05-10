"$REF"  - <<<'{"foo":1,"bar":2}' > "$TMPDIR/expected" 2>&1 || true
"$RUST" - <<<'{"foo":1,"bar":2}' > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump - <<<'{...}' (stdin parses non-empty input)"
