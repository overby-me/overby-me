"$REF" --help > "$TMPDIR/expected" 2>&1
"$RUST" --help > "$TMPDIR/actual" 2>&1
compare "pipewire-vulkan/help"
