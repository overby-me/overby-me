# spa-acp-tool doesn't declare --version. Long-form `--version` falls
# through to the getopt "unrecognized option" path.
"$REF"  --version </dev/null > "$TMPDIR/c.full" 2>&1 || true
"$RUST" --version </dev/null > "$TMPDIR/r.full" 2>&1 || true
filter() {
  grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' "$1" > "$2" || true
}
filter "$TMPDIR/c.full" "$TMPDIR/expected"
filter "$TMPDIR/r.full" "$TMPDIR/actual"
compare "spa-acp-tool --version (unrecognized: not declared)"
