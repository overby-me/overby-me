# pw-config -L paths: omit newlines.
mkdir -p "$TMPDIR/conf-dir"
cat > "$TMPDIR/conf-dir/pipewire.conf" <<EOF
context.properties = { core.daemon = true }
EOF

export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf-dir"
export HOME="$TMPDIR/no-home"
export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

"$REF" -L paths > "$TMPDIR/expected" 2>/dev/null
"$RUST" -L paths > "$TMPDIR/actual" 2>/dev/null
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-config/paths-no-newline"
