# pw-config -n custom.conf paths
mkdir -p "$TMPDIR/conf-dir"
cat > "$TMPDIR/conf-dir/jack.conf" <<EOF
jack.properties = { foo = bar }
EOF

export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf-dir"
export HOME="$TMPDIR/no-home"
export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

"$REF" -n jack.conf paths > "$TMPDIR/expected" 2>/dev/null
"$RUST" -n jack.conf paths > "$TMPDIR/actual" 2>/dev/null
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-config/paths-custom-name"
