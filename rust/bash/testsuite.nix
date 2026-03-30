# Run a single test from the official GNU Bash test suite against rust-bash.
#
# Compares rust-bash output against reference bash output (both running in the
# same sandbox), avoiding false failures from .right files generated on
# different systems.
#
# Run with: nix build .#checks.x86_64-linux.rust-bash-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-bash-test-arith
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-bash-test-${name}" {
  nativeBuildInputs = [pkgs.rust-bash-dev pkgs.bash pkgs.gcc pkgs.coreutils pkgs.diffutils pkgs.gnused pkgs.gnugrep pkgs.gawk pkgs.findutils pkgs.glibcLocales];
  bashSrc = pkgs.bash.src;
} ''
  # Extract the test suite and helper sources
  tar xzf $bashSrc
  BASH_SRC=$(echo bash-*)

  # Build helper programs (recho, zecho, printenv) needed by the tests
  gcc -o recho "$BASH_SRC/support/recho.c"
  gcc -o zecho "$BASH_SRC/support/zecho.c"
  gcc -o printenv "$BASH_SRC/support/printenv.c"
  gcc -o xcase "$BASH_SRC/support/xcase.c"

  cd "$BASH_SRC/tests"

  # Put helpers on PATH
  export PATH="$OLDPWD:$PATH"
  export TMPDIR="$(mktemp -d)"

  # Make locales available for tests that need them (e.g. printf2.sub)
  export LOCALE_ARCHIVE="${pkgs.glibcLocales}/lib/locale/locale-archive"

  echo "Running bash test: ${name}"

  # Run with reference bash
  export THIS_SH="${pkgs.bash}/bin/bash"
  timeout 300 "$THIS_SH" "./${name}.tests" > "$TMPDIR/expected" 2>&1 || true

  # Run with rust-bash
  export THIS_SH="${pkgs.rust-bash-dev}/bin/bash"
  timeout 300 "$THIS_SH" "./${name}.tests" > "$TMPDIR/actual" 2>&1 || true

  # Normalize binary paths so that /nix/store/.../bin/bash differences don't
  # cause false failures. Replace both shell paths with a generic "bash" prefix.
  REF_BASH="${pkgs.bash}/bin/bash"
  TEST_BASH="${pkgs.rust-bash-dev}/bin/bash"
  sed -i "s|$REF_BASH|bash|g" "$TMPDIR/expected"
  sed -i "s|$TEST_BASH|bash|g" "$TMPDIR/actual"

  # Normalize PIDs in temp paths used by tests.  Tests create paths like
  # /tmp-<pid>, /type-<pid>, /bash-zzz-<pid>, /zero-length-file-<pid>.
  # Match: a word-hyphen-digits pattern where the digits are 2-7 long.
  sed -i -E 's|([a-zA-Z])-([0-9]{2,7})\b|\1-PID|g' "$TMPDIR/expected" "$TMPDIR/actual"

  # Normalize thread/process IDs in Rust panic messages
  sed -i -E "s|thread '([^']*)' \([0-9]+\)|thread '\1' (PID)|g" "$TMPDIR/expected" "$TMPDIR/actual"

  # Compare
  if diff --text "$TMPDIR/actual" "$TMPDIR/expected"; then
    touch $out
  else
    exit 1
  fi
''
