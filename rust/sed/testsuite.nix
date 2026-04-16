# Run a single .sh test from the official GNU sed 4.9 test suite against rust-sed.
#
# The test scripts use gnulib's init.sh framework with helpers like compare_,
# returns_, skip_, fail_, and Exit. They expect TESTS_ENVIRONMENT variables
# set by the Makefile (abs_top_srcdir, abs_top_builddir, etc.).
#
# Run with: nix build .#checks.x86_64-linux.rust-sed-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-sed-test-subst-options
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-sed-test-${name}" {
  nativeBuildInputs = [
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused # needed by init.cfg's remove_cr_inplace
    pkgs.gnugrep
    pkgs.bash
    pkgs.gawk # some tests use AWK variable
    pkgs.perl # for get-mb-cur-max helper
  ];
  gnusedSrc = pkgs.gnused.src;
  rustSed = pkgs.rust-sed-dev;
} ''
  # Extract the GNU sed source (contains the test suite)
  tar xf $gnusedSrc
  SED_SRC=$(cd sed-* && pwd)

  # Create a directory that mimics the build tree layout expected by the tests.
  # Tests do: path_prepend_ ./sed  (prepends $PWD/sed to PATH)
  mkdir -p "$SED_SRC/sed"
  ln -s "$rustSed/bin/sed" "$SED_SRC/sed/sed"

  # Also create sed symlink in /tmp/sed/ for tests that use ../sed/sed
  # from temp dirs created in /tmp by init.sh's mktempd_
  mkdir -p /tmp/sed
  ln -s "$rustSed/bin/sed" /tmp/sed/sed

  cd "$SED_SRC"

  # Set up TESTS_ENVIRONMENT variables (from testsuite/local.mk)
  export srcdir="."
  export top_srcdir="."
  export abs_top_srcdir="$SED_SRC"
  export abs_top_builddir="$SED_SRC"
  export abs_srcdir="$SED_SRC"
  export LC_ALL=C
  export VERSION="4.9"
  export PACKAGE_VERSION="4.9"
  export AWK="${pkgs.gawk}/bin/gawk"
  export PERL="${pkgs.perl}/bin/perl"
  export built_programs="sed"
  export PATH="$SED_SRC/sed:$PATH"

  # Run the test script; exit codes: 0=pass, 77=skip, other=fail
  # Initialize fail=0 to avoid "integer expected" in tests that check $fail
  if fail=0 bash ${if name == "compile-errors" then "-x" else ""} "testsuite/${name}.sh"; then
    touch $out
  else
    rc=$?
    if [ "$rc" = "77" ]; then
      echo "SKIP: test ${name} was skipped"
      touch $out
    else
      echo "FAIL: test ${name} exited with code $rc"
      exit 1
    fi
  fi
''
