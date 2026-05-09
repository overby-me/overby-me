"$REF" -V > "$TMPDIR/expected" 2>&1
"$RUST" -V > "$TMPDIR/actual" 2>&1
compare "pipewire-vulkan/version-short"
