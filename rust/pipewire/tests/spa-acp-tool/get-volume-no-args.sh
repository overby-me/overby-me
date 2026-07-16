"$REF"  get-volume </dev/null > "$TMPDIR/c.full" 2>&1 || true
"$RUST" get-volume </dev/null > "$TMPDIR/r.full" 2>&1 || true
filter() {
  grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' "$1" > "$2" || true
}
filter "$TMPDIR/c.full" "$TMPDIR/expected"
filter "$TMPDIR/r.full" "$TMPDIR/actual"
compare "spa-acp-tool/get-volume-no-args"
