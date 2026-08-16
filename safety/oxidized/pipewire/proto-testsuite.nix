# Daemon-interop test: spawns a real C pipewire daemon and probes it via
# our Rust protocol-native client.
{
  pkgs,
  name,
}:
pkgs.runCommand "oxidized-pipewire-proto-test-${name}" {
  nativeBuildInputs = [
    pkgs.oxidized-pipewire-dev
    pkgs.pipewire
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.nushell
  ];
  testScript = ./tests/proto/${name}.nu;
} ''
  export TMPDIR="$(mktemp -d)"
  export LC_ALL=C

  pipewireBin="${pkgs.pipewire}"
  export pipewireBin

  # The "RUST" binary in this harness is oxidized-pipewire's multicall;
  # the test script invokes it as `$RUST proto-probe ...`.
  RUST="${pkgs.oxidized-pipewire-dev}/bin/oxidized-pipewire"
  export RUST

  nu "$testScript" || exit 1

  touch $out
''
