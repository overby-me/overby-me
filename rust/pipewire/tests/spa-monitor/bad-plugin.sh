# Both tools exit 255 on dlopen failure. Use `|| true` so set -e in
# the nix builder doesn't abort the script before we reach compare().
"$REF" /nonexistent.so > "$TMPDIR/expected" 2>&1 || true
"$RUST" /nonexistent.so > "$TMPDIR/actual" 2>&1 || true
compare "spa-monitor/bad-plugin"
