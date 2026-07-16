"$REF" "$SRC/src/daemon/pipewire-vulkan.conf.in" > "$TMPDIR/expected" 2>&1
"$RUST" "$SRC/src/daemon/pipewire-vulkan.conf.in" > "$TMPDIR/actual" 2>&1
compare "spa-json-dump/conf-pipewire-vulkan"
