# Run a single test from the flatpak test suite against rust-flatpak.
#
# Each test script receives $FLATPAK, $WORK, and $HOME pointing to
# a writable sandbox. Exit 0 = pass, non-zero = fail.
#
# Run with: nix build .#checks.x86_64-linux.rust-flatpak-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-flatpak-test-version
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-flatpak-test-${name}" {
  nativeBuildInputs = [pkgs.rust-flatpak-dev pkgs.coreutils pkgs.gnugrep pkgs.gnused pkgs.diffutils pkgs.bash pkgs.nushell];
} ''
  export WORK="$(mktemp -d)"
  export HOME="$WORK/home"
  mkdir -p "$HOME/.local/share/flatpak"
  export FLATPAK="${pkgs.rust-flatpak-dev}/bin/flatpak"

  echo "Running flatpak test: ${name}"

  nu ${./tests/${name}.nu}

  touch $out
''
