# End-to-end test: rust-meson init → setup → ninja build → run hello world.
#
# Run with: nix build .#checks.x86_64-linux.rust-meson-hello-world
{pkgs}:
pkgs.runCommand "rust-meson-hello-world" {
  nativeBuildInputs = [pkgs.rust-meson pkgs.ninja pkgs.gcc];
} ''
  # Create a temporary project
  mkdir project && cd project
  meson init --name hello --language c

  # Configure
  meson setup builddir

  # Build
  ninja -C builddir

  # Run and verify output
  output=$(./builddir/hello)
  echo "Program output: $output"
  test "$output" = "Hello, world!"

  # Success
  touch $out
''
