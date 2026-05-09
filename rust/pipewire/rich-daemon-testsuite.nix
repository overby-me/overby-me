# Daemon-comparison tests against a richer daemon — one with a
# `support.null-audio-sink` Node loaded so we can exercise Node-related
# code paths (Node.Info events, Node ls, Node info).
{
  pkgs,
  tool,
  name,
}:
pkgs.runCommand "rust-pipewire-rich-daemon-test-${tool}-${name}" {
  nativeBuildInputs = [
    pkgs.rust-pipewire-dev
    pkgs.pipewire
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.gawk
  ];
  testScript = ./tests/${tool}/${name}.sh;
} ''
  export TMPDIR="$(mktemp -d)"
  export LC_ALL=C
  export LANG=C
  export LC_NUMERIC=C

  export XDG_RUNTIME_DIR="$TMPDIR/run"
  mkdir -p "$XDG_RUNTIME_DIR"
  chmod 700 "$XDG_RUNTIME_DIR"

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
      { name = libpipewire-module-access }
      # Need the spa-node-factory to instantiate the null-audio-sink below.
      { name = libpipewire-module-spa-node-factory }
      # Same reason as in daemon-testsuite.nix: pw-cli auto-binds every
      # global it sees, and a missing metadata factory writes a noisy
      # error to stderr that breaks comparison.
      { name = libpipewire-module-metadata }
  ]
  context.objects = [
      {   factory = spa-node-factory
          args = {
              factory.name = support.null-audio-sink
              node.name    = test-null-sink
              media.class  = "Audio/Sink"
              audio.position = "FL,FR"
          }
      }
  ]
  context.exec = []
  EOF

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
  export PIPEWIRE_REMOTE=pipewire-test
  export PIPEWIRE_CONFIG_DIR="$TMPDIR/conf"
  export HOME="$TMPDIR/no-home"
  export XDG_CONFIG_HOME="$TMPDIR/no-xdg"

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
    cat "$TMPDIR/daemon.log" || true
    exit 1
  fi

  # Give the daemon a moment to instantiate the configured null-audio-sink.
  sleep 0.5

  binName="${tool}"
  export REF="${pkgs.pipewire}/bin/$binName"
  export RUST="${pkgs.rust-pipewire-dev}/bin/$binName"

  compare() {
    local label="$1"
    local ref_out="$TMPDIR/expected"
    local rust_out="$TMPDIR/actual"
    sed -i -E 's|/nix/store/[a-z0-9]{32}-[^/[:space:]]+/bin/[^[:space:]]+|TOOL|g' \
      "$ref_out" "$rust_out"
    sed -i -E 's|/nix/store/[a-z0-9]{32}-[^[:space:]]+|NIXPATH|g' \
      "$ref_out" "$rust_out"
    sed -i 's/[[:space:]]*$//' "$ref_out" "$rust_out"
    if diff --text "$rust_out" "$ref_out"; then
      echo "PASS: $label"
    else
      echo "FAIL: $label"
      echo "--- expected ---"
      cat "$ref_out"
      echo "--- actual ---"
      cat "$rust_out"
      exit 1
    fi
  }

  echo "Running rich-daemon test: ${tool}/${name}"
  source "$testScript"

  touch $out
''
