"$REF" "$SRC/src/daemon/pipewire-pulse.conf.in" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/pipewire-pulse.conf.in" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-pipewire-pulse"
