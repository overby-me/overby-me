# Daemon-interop test: spawns a real C pipewire daemon and probes it via
# our Rust protocol-native client.
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-pipewire-proto-test-${name}" {
  nativeBuildInputs = [
    pkgs.rust-pipewire-dev
    pkgs.pipewire
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.gnused
  ];
  testScript = ./tests/proto/${name}.sh;
} ''
  export TMPDIR="$(mktemp -d)"
  export LC_ALL=C

  pipewireBin="${pkgs.pipewire}"
  export pipewireBin

  # The "RUST" binary in this harness is rust-pipewire's multicall;
  # the test script invokes it as `$RUST proto-probe ...`.
  RUST="${pkgs.rust-pipewire-dev}/bin/rust-pipewire"
  export RUST

  bash "$testScript" || exit 1

  touch $out
''
