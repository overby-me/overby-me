# pw-link with no daemon socket: C prints `can't connect: Host is down`
# (no `Error:` wrapper — different from pw-cli) and exits 255.
unset PIPEWIRE_REMOTE
unset PIPEWIRE_CORE
"$REF"  -r /nonexistent -i </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -r /nonexistent -i </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link connect-fail"
