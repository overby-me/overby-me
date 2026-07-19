# Daemon-comparison tests: spawns a real C pipewire daemon, then runs both
# the C reference tool and rust-pipewire's tool against it and diffs the
# output. This is the M7 test pattern: each tool's client implementation
# is verified end-to-end against a real server.
{
  pkgs,
  tool,
  name,
}:
pkgs.runCommand "rust-pipewire-daemon-test-${tool}-${name}" {
  nativeBuildInputs = [
    pkgs.rust-pipewire-dev
    pkgs.pipewire
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.nushell
  ];
  testScript = ./tests/${tool}/${name}.nu;
  testHelpers = ./tests/helpers.nu;
} ''
  export TMPDIR="$(mktemp -d)"
  export LC_ALL=C
  export LANG=C
  export LC_NUMERIC=C

  # Private XDG_RUNTIME_DIR so the daemon socket is isolated.
  export XDG_RUNTIME_DIR="$TMPDIR/run"
  mkdir -p "$XDG_RUNTIME_DIR"
  chmod 700 "$XDG_RUNTIME_DIR"

  # Minimal daemon config: only protocol-native + client-node, no audio
  # backends, no rtkit, no D-Bus. The configured core.name lets the test
  # connect to a fixed socket path.
  mkdir -p "$TMPDIR/conf"
  cat > "$TMPDIR/conf/pipewire.conf" <<'EOF'
  context.properties = {
      core.daemon = true
      core.name   = pipewire-test
      log.level   = 2
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
      # The access module grants PW_PERM_ALL to non-sandboxed clients —
      # without it the registry has zero visible globals because every
      # global's read permission check fails.
      { name = libpipewire-module-access }
      # The C pw-cli auto-binds every global it sees on the registry. If
      # the daemon doesn't ship a metadata factory, that bind fails with
      # `remote 0: error ... can't bind global N/3: -71` and pollutes
      # stderr, breaking our diff-based comparison tests.
      { name = libpipewire-module-metadata }
  ]
  context.objects = []
  context.exec = []
  EOF

  # Client config — the C tool's `pw_context_new` requires this to exist
  # (it bails with `failed to connect: ...` otherwise) and crashes if any
  # of the standard modules are missing. Mirror the upstream-shipped
  # `client.conf` shape but disable everything that needs hardware/D-Bus.
  cat > "$TMPDIR/conf/client.conf" <<'EOF'
  context.properties = { log.level = 0 }
  context.spa-libs = {
      audio.convert.* = audioconvert/libspa-audioconvert
      support.*       = support/libspa-support
      video.convert.* = videoconvert/libspa-videoconvert
  }
  context.modules = [
      { name = libpipewire-module-rt flags = [ ifexists nofail ] condition = [ { module.rt = !false } ] }
      { name = libpipewire-module-protocol-native }
      { name = libpipewire-module-client-node condition = [ { module.client-node = !false } ] }
      { name = libpipewire-module-client-device condition = [ { module.client-device = !false } ] }
      { name = libpipewire-module-adapter condition = [ { module.adapter = !false } ] }
      { name = libpipewire-module-metadata condition = [ { module.metadata = !false } ] }
      { name = libpipewire-module-session-manager condition = [ { module.session-manager = !false } ] }
  ]
  EOF

  export PIPEWIRE_CORE=pipewire-test
  # Client side also reads PIPEWIRE_REMOTE (the C tool's `pw-cli`
  # `pw_context_connect` path); both env vars need to point at our private
  # daemon name.
  export PIPEWIRE_REMOTE=pipewire-test
  export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf"
  export HOME="$TMPDIR/no-home"
  export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

  # Reference C daemon.
  DAEMON="${pkgs.pipewire}/bin/pipewire"
  "$DAEMON" >"$TMPDIR/daemon.log" 2>&1 &
  DAEMON_PID=$!
  trap 'kill -TERM $DAEMON_PID 2>/dev/null; wait $DAEMON_PID 2>/dev/null' EXIT

  for i in $(seq 1 50); do
    if [ -S "$XDG_RUNTIME_DIR/pipewire-test" ]; then
      break
    fi
    sleep 0.1
  done
  if [ ! -S "$XDG_RUNTIME_DIR/pipewire-test" ]; then
    echo "FAIL: daemon socket never appeared"
    echo "--- daemon log ---"
    cat "$TMPDIR/daemon.log" || true
    echo "--- runtime dir ---"
    ls -la "$XDG_RUNTIME_DIR" || true
    echo "--- env ---"
    env | grep -E '^(PIPEWIRE|XDG)' || true
    exit 1
  fi
  echo "daemon: socket appeared at $XDG_RUNTIME_DIR/pipewire-test"

  # Tool-name → binary mapping (same in both packages).
  binName="${tool}"
  export REF="${pkgs.pipewire}/bin/$binName"
  export RUST="${pkgs.rust-pipewire-dev}/bin/$binName"

  # Lay the fixture out one directory below helpers.nu, mirroring the
  # repository layout so the fixture's `source ../helpers.nu` resolves.
  cp $testHelpers helpers.nu
  mkdir fixture
  cp $testScript fixture/test.nu

  echo "Running daemon test: ${tool}/${name}"
  nu fixture/test.nu

  touch $out
''
