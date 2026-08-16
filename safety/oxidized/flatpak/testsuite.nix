# Run a single test from the flatpak test suite against oxidized-flatpak.
#
# Each test script receives $FLATPAK, $WORK, and $HOME pointing to
# a writable sandbox. Exit 0 = pass, non-zero = fail.
#
# Run with: nix build .#checks.x86_64-linux.oxidized-flatpak-test-{name}
# Example:  nix build .#checks.x86_64-linux.oxidized-flatpak-test-version
{
  pkgs,
  name,
}:
pkgs.runCommand "oxidized-flatpak-test-${name}" {
  nativeBuildInputs = [pkgs.oxidized-flatpak-dev pkgs.coreutils pkgs.gnugrep pkgs.gnused pkgs.diffutils pkgs.bash pkgs.nushell];
} ''
  export WORK="$(mktemp -d)"
  export HOME="$WORK/home"
  mkdir -p "$HOME/.local/share/flatpak"
  export FLATPAK="${pkgs.oxidized-flatpak-dev}/bin/flatpak"

  echo "Running flatpak test: ${name}"

  nu ${./tests/${name}.nu}

  touch $out
''
