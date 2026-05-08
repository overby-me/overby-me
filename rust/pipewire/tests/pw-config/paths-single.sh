# pw-config paths with a single config file (no overrides).
mkdir -p "$TMPDIR/conf-dir"
cat > "$TMPDIR/conf-dir/pipewire.conf" <<EOF
context.properties = {
    core.daemon = true
    core.name = pipewire-0
}
EOF

# Force the C tool to only see PIPEWIRE_CONFIG_DIR; HOME/XDG_CONFIG_HOME
# defaults would otherwise interfere.
export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf-dir"
export HOME="$TMPDIR/no-home"
export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

"$REF" > "$TMPDIR/expected" 2>/dev/null
"$RUST" > "$TMPDIR/actual" 2>/dev/null
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-config/paths-single"
