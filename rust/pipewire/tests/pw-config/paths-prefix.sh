# pw-config -p prefix.
mkdir -p "$TMPDIR/conf-dir"
cat > "$TMPDIR/conf-dir/pipewire.conf" <<INNER
context.properties = { core.daemon = true }
INNER

export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf-dir"
export HOME="$TMPDIR/no-home"
export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

"$REF" -p TEST paths > "$TMPDIR/expected" 2>/dev/null
"$RUST" -p TEST paths > "$TMPDIR/actual" 2>/dev/null
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-config/paths-prefix"
