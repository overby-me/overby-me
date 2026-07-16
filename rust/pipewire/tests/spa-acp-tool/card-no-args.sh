"$REF"  card </dev/null > "$TMPDIR/c.full" 2>&1 || true
"$RUST" card </dev/null > "$TMPDIR/r.full" 2>&1 || true
# Filter ALSA-init noise C emits in the sandbox where no real cards
# are present (do_probe runs before the command and logs warnings).
filter() {
  grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' "$1" > "$2" || true
}
filter "$TMPDIR/c.full" "$TMPDIR/expected"
filter "$TMPDIR/r.full" "$TMPDIR/actual"
compare "spa-acp-tool/card-no-args"
