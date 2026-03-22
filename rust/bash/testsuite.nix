# Run a single test from the official GNU Bash test suite against rust-bash.
#
# Run with: nix build .#checks.x86_64-linux.rust-bash-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-bash-test-arith
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-bash-test-${name}" {
  nativeBuildInputs = [pkgs.rust-bash pkgs.gcc pkgs.coreutils pkgs.diffutils pkgs.gnused pkgs.gnugrep pkgs.gawk pkgs.findutils];
  bashSrc = pkgs.bash.src;
} ''
  # Extract the test suite and helper sources
  tar xzf $bashSrc
  BASH_SRC=$(echo bash-*)

  # Build helper programs (recho, zecho, printenv) needed by the tests
  gcc -o recho "$BASH_SRC/support/recho.c"
  gcc -o zecho "$BASH_SRC/support/zecho.c"
  gcc -o printenv "$BASH_SRC/support/printenv.c"

  cd "$BASH_SRC/tests"

  # Put helpers and rust-bash on PATH
  export PATH="$OLDPWD:${pkgs.rust-bash}/bin:$PATH"
  export THIS_SH="${pkgs.rust-bash}/bin/bash"
  export TMPDIR="$(mktemp -d)"
  export BASH_TSTOUT="$TMPDIR/bashtst-$$"

  echo "Running bash test: ${name}"
  timeout 60 sh "run-${name}"

  touch $out
''
