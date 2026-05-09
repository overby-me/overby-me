mkdir -p "$TMPDIR/conf-dir"
cat > "$TMPDIR/conf-dir/pipewire.conf" <<INNER
context.properties = { core.daemon = true }
INNER

export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf-dir"
export HOME="$TMPDIR/no-home"
export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

"$REF"  -L merge bad </dev/null > "$TMPDIR/expected" 2>/dev/null || true
"$RUST" -L merge bad </dev/null > "$TMPDIR/actual"  2>/dev/null || true
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-config -L merge bad (empty container '{ }')"
