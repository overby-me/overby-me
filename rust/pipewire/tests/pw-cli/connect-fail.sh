# pw-cli with no daemon socket: the C tool's protocol-native client maps
# the underlying ENOENT to EHOSTDOWN, then parse() wraps the message in
# `Error: "..."`. Unset both PIPEWIRE_REMOTE and PIPEWIRE_CORE env vars
# (the daemon-test wrapper sets them) and explicitly point at a path
# that doesn't exist.
unset PIPEWIRE_REMOTE
unset PIPEWIRE_CORE
"$REF"  -r /nonexistent ls Core </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -r /nonexistent ls Core </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli connect-fail"
