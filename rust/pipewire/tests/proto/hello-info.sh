# Spawn a real C pipewire daemon, connect with our Rust protocol-native
# client, and verify we receive a Core.Info plus at least one Registry
# Global event. This proves wire-format compatibility end-to-end.

# Set up a private XDG_RUNTIME_DIR so the daemon's socket is isolated.
export XDG_RUNTIME_DIR="$TMPDIR/run"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

# Minimal daemon config — only the protocol-native module so we don't
# pull in audio backends (no /dev/snd in the sandbox).
mkdir -p "$TMPDIR/conf"
cat > "$TMPDIR/conf/pipewire.conf" <<'EOF'
context.properties = {
    core.daemon = true
    core.name   = pipewire-test
    log.level   = 0
    mem.warn-mlock = false
    mem.allow-mlock = false
    support.dbus   = false
}
context.spa-libs = {
    support.* = support/libspa-support
}
context.modules = [
    { name = libpipewire-module-protocol-native }
    { name = libpipewire-module-client-node }
]
context.objects = []
context.exec = []
EOF

export PIPEWIRE_CORE=pipewire-test
export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf"
export HOME="$TMPDIR/no-home"
export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

# Use the upstream C pipewire daemon, not our symlinked Rust stub.
DAEMON="$pipewireBin/bin/pipewire"
if [ ! -x "$DAEMON" ]; then
  echo "FAIL: pipewire daemon not found at $DAEMON"
  exit 1
fi

# Spawn the daemon.
"$DAEMON" >"$TMPDIR/daemon.log" 2>&1 &
DAEMON_PID=$!
trap 'kill -TERM $DAEMON_PID 2>/dev/null; wait $DAEMON_PID 2>/dev/null' EXIT

# Wait up to 5 seconds for the socket to appear.
for i in $(seq 1 50); do
  if [ -S "$XDG_RUNTIME_DIR/pipewire-test" ]; then
    break
  fi
  sleep 0.1
done
if [ ! -S "$XDG_RUNTIME_DIR/pipewire-test" ]; then
  echo "FAIL: daemon socket never appeared"
  echo "daemon log:"; cat "$TMPDIR/daemon.log"
  exit 1
fi

# Run our client.
"$RUST" proto-probe 30 > "$TMPDIR/probe.out" 2>&1
exit_code=$?
echo "probe exit: $exit_code"

# The probe output should contain at least one Core.Info (id=0 opcode=0)
# and at least one Registry.Global (id=1 opcode=0) event.
core_info_lines=$(grep -c '^msg id=0 opcode=0 ' "$TMPDIR/probe.out" || true)
registry_lines=$(grep -c '^msg id=1 opcode=0 ' "$TMPDIR/probe.out" || true)

echo "probe output (first 5 messages):"
head -5 "$TMPDIR/probe.out" | sed 's/^/  /'

if [ "$core_info_lines" -lt 1 ]; then
  echo "FAIL: no Core.Info received"
  echo "daemon log:"; cat "$TMPDIR/daemon.log"
  exit 1
fi
if [ "$registry_lines" -lt 1 ]; then
  echo "FAIL: no Registry events received"
  echo "daemon log:"; cat "$TMPDIR/daemon.log"
  exit 1
fi

echo "PASS: proto/hello-info ($core_info_lines Core.Info, $registry_lines Registry events)"
