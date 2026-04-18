# Run a single test from the official GNU gzip test suite against rust-gzip.
#
# The upstream tests are self-checking POSIX sh scripts that source
# tests/init.sh and signal pass/fail via exit code. We just need to stage
# a tree where `gzip`, `gunzip`, `zcat` resolve to rust-gzip and the
# companion shell scripts (`zdiff`, `zgrep`, ...) resolve to the upstream
# versions shipped by pkgs.gzip — those scripts call `gzip` by name, so
# they pick up our binary from PATH.
#
# Run with: nix build .#checks.x86_64-linux.rust-gzip-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-gzip-test-keep
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-gzip-test-${name}" {
  nativeBuildInputs = [
    pkgs.rust-gzip-dev
    pkgs.gzip
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused
    pkgs.gnugrep
    pkgs.bash
    pkgs.findutils
    pkgs.gawk
  ];
  gzipSrc = pkgs.gzip.src;
} ''
  # Extract the test suite.
  tar xf $gzipSrc
  GZIP_SRC=$(echo gzip-*)
  cd "$GZIP_SRC"

  # Upstream tests do `path_prepend_ ..` which puts the extracted source
  # root onto $PATH. Populate that directory with rust-gzip (for gzip,
  # gunzip, zcat) and upstream companion scripts (for zdiff, zgrep, ...).
  # The companion scripts call `gzip` by name, so they pick up rust-gzip.
  for b in gzip gunzip zcat; do
    ln -sf ${pkgs.rust-gzip-dev}/bin/$b ./$b
  done
  for b in zdiff zcmp zegrep zfgrep zforce zgrep znew zless zmore gzexe; do
    if [ -x ${pkgs.gzip}/bin/$b ]; then
      ln -sf ${pkgs.gzip}/bin/$b ./$b
    fi
  done

  cd tests

  export TMPDIR="$(mktemp -d)"
  export LC_ALL=C
  export srcdir=.
  export abs_srcdir="$PWD"
  export top_srcdir="$PWD/.."
  export abs_top_srcdir="$PWD/.."
  export abs_top_builddir="$PWD/.."
  export VERSION="${pkgs.gzip.version}"
  export PACKAGE_VERSION="${pkgs.gzip.version}"
  export PACKAGE_BUGREPORT="bug-gzip@gnu.org"
  export GREP="${pkgs.gnugrep}/bin/grep"
  export SHELL="${pkgs.bash}/bin/sh"
  export TERM=dumb
  unset PAGER
  export EXEEXT=""
  export built_programs="gzip gunzip zcat zdiff zcmp zegrep zfgrep zforce zgrep znew gzexe"
  export PATH="$abs_top_builddir:$PATH"

  echo "Running gzip test: ${name}"
  # Makefile.am's TESTS_ENVIRONMENT ends with `; 9>&2`, which init.cfg
  # reads as `stderr_fileno_=9`. Provide it here so init.sh's skip_/
  # framework_failure_ helpers don't trip on "Bad file descriptor".
  if timeout 60 sh "./${name}" 9>&2; then
    touch $out
  else
    exit 1
  fi
''
