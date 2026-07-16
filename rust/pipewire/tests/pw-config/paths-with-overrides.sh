# pw-config paths with main + drop-in overrides in `pipewire.conf.d/`.
mkdir -p "$TMPDIR/conf-dir/pipewire.conf.d"
cat > "$TMPDIR/conf-dir/pipewire.conf" <<EOF
context.properties = { core.daemon = true }
EOF
cat > "$TMPDIR/conf-dir/pipewire.conf.d/00-first.conf" <<EOF
context.properties = { core.name = pipewire-0 }
EOF
cat > "$TMPDIR/conf-dir/pipewire.conf.d/99-last.conf" <<EOF
context.properties = { log.level = 3 }
EOF

export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf-dir"
export HOME="$TMPDIR/no-home"
export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

"$REF" > "$TMPDIR/expected" 2>/dev/null
"$RUST" > "$TMPDIR/actual" 2>/dev/null
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-config/paths-with-overrides"
