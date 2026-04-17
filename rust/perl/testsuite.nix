# Run a single test from the upstream Perl test suite against rust-perl.
#
# Compares rust-perl output against reference perl output (both running in the
# same sandbox), avoiding false failures from system-specific differences.
#
# Run with: nix build .#checks.x86_64-linux.rust-perl-test-{category}-{name}
# Example:  nix build .#checks.x86_64-linux.rust-perl-test-base-if
# View log: nix log .#checks.x86_64-linux.rust-perl-test-base-if
{
  pkgs,
  category,
  name,
}:
pkgs.runCommand "rust-perl-test-${category}-${name}" {
  nativeBuildInputs = [pkgs.rust-perl-dev pkgs.perl pkgs.coreutils pkgs.diffutils pkgs.gnused pkgs.gnugrep];
  perlSrc = pkgs.perl.src;
} ''
  # Extract the perl source
  tar xf $perlSrc
  PERL_SRC=$(echo perl-*)

  cd "$PERL_SRC/t"

  export TMPDIR="$(mktemp -d)"

  echo "Running perl test: ${category}/${name}"

  # Run with reference perl — execute the .t file and capture TAP output
  timeout 60 ${pkgs.perl}/bin/perl -I../lib ${category}/${name}.t > "$TMPDIR/expected" 2>&1 || true

  # Run with rust-perl
  timeout 60 ${pkgs.rust-perl-dev}/bin/perl -I../lib ${category}/${name}.t > "$TMPDIR/actual" 2>&1 || true

  # Normalize binary paths so /nix/store/... differences don't cause false failures
  REF_PERL="${pkgs.perl}/bin/perl"
  TEST_PERL="${pkgs.rust-perl-dev}/bin/perl"
  sed -i "s|$REF_PERL|perl|g" "$TMPDIR/expected"
  sed -i "s|$TEST_PERL|perl|g" "$TMPDIR/actual"

  # Normalize any remaining nix store paths
  sed -i -E 's|/nix/store/[a-z0-9]{32}-[^/]+|NIXPATH|g' "$TMPDIR/expected" "$TMPDIR/actual"

  # Compare
  if diff --text "$TMPDIR/actual" "$TMPDIR/expected"; then
    touch $out
  else
    exit 1
  fi
''
