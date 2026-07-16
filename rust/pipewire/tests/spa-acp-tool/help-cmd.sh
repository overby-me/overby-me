# `help` as a subcommand (vs --help flag) lists the spa-acp-tool
# interactive command set. Same output as the bare REPL prompt's
# help, exercised here once at startup.
"$REF"  help </dev/null > "$TMPDIR/c.full" 2>&1 || true
"$RUST" help </dev/null > "$TMPDIR/r.full" 2>&1 || true
filter() {
  grep -v '^E       alsa-ucm.c\|^W      alsa-util.c\|^E            acp.c' "$1" > "$2" || true
}
filter "$TMPDIR/c.full" "$TMPDIR/expected"
filter "$TMPDIR/r.full" "$TMPDIR/actual"
compare "spa-acp-tool help (subcommand prints interactive command list)"
