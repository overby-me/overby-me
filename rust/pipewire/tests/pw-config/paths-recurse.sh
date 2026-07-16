# pw-config -r paths: recurse into nested objects (currently unused
# for the simple paths case but mirrors C's flag handling).
mkdir -p "$TMPDIR/conf-dir"
cat > "$TMPDIR/conf-dir/pipewire.conf" <<INNER
context.properties = { core.daemon = true }
INNER

export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf-dir"
export HOME="$TMPDIR/no-home"
export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

"$REF" -r paths > "$TMPDIR/expected" 2>/dev/null
"$RUST" -r paths > "$TMPDIR/actual" 2>/dev/null
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-config/paths-recurse"
