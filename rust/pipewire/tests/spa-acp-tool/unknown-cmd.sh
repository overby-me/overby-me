"$REF"  foo </dev/null > "$TMPDIR/c.full" 2>&1 || true
"$RUST" foo </dev/null > "$TMPDIR/r.full" 2>&1 || true
# Filter ALSA-init noise C emits in the sandbox.
filter() {
  grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' "$1" > "$2"
}
filter "$TMPDIR/c.full" "$TMPDIR/expected"
filter "$TMPDIR/r.full" "$TMPDIR/actual"
compare "spa-acp-tool foo (unknown command)"
