# Run a single test from the official curl test suite against rust-curl.
#
# Uses curl's runtests.pl with the -c flag to test an alternate binary.
# The C curl project is built first to provide the test servers and
# infrastructure, then runtests.pl runs the specified test against rust-curl.
#
# Run with: nix build .#checks.x86_64-linux.rust-curl-test-{num}
# Example:  nix build .#checks.x86_64-linux.rust-curl-test-1
{
  pkgs,
  testNum,
}: let
  testNumStr = toString testNum;

  # Build the curl C project to get test servers and infrastructure.
  curl-test-infra = pkgs.stdenv.mkDerivation {
    pname = "curl-test-infra";
    inherit (pkgs.curl) version src;

    nativeBuildInputs = with pkgs; [
      autoreconfHook
      pkg-config
      perl
      python3
    ];

    buildInputs = with pkgs; [
      openssl
      zlib
      nghttp2
      libpsl
    ];

    # Patch shebangs in scripts (fixes /usr/bin/env issues)
    postPatch = ''
      patchShebangs scripts/
    '';

    configureFlags = [
      "--with-openssl"
      "--without-libssh2"
      "--disable-ldap"
      "--without-brotli"
      "--without-zstd"
      "--without-librtmp"
      "--without-libidn2"
      "--disable-docs"
    ];

    # Build everything needed for tests
    buildPhase = ''
      make -C lib -j$NIX_BUILD_CORES
      make -C src -j$NIX_BUILD_CORES
      make -C tests -j$NIX_BUILD_CORES
    '';

    installPhase = ''
      mkdir -p $out/lib $out/src

      # Copy test infrastructure
      cp -r tests $out/tests

      # Copy the C curl binary and curlinfo (needed by runtests.pl)
      cp src/.libs/curl $out/src/curl 2>/dev/null || cp src/curl $out/src/curl
      if [ -f src/.libs/curlinfo ]; then
        cp src/.libs/curlinfo $out/src/curlinfo
      elif [ -f src/curlinfo ]; then
        cp src/curlinfo $out/src/curlinfo
      fi

      # Copy libcurl for test servers and curlinfo
      cp lib/.libs/libcurl.so* $out/lib/ 2>/dev/null || true

      # Make runtests.pl executable
      chmod +x $out/tests/runtests.pl
    '';

    dontStrip = true;
  };
in
  pkgs.runCommand "rust-curl-test-${testNumStr}" {
    nativeBuildInputs = [
      pkgs.rust-curl-dev
      pkgs.perl
      pkgs.coreutils
      pkgs.diffutils
      pkgs.gnused
      pkgs.gnugrep
      pkgs.stunnel
    ];
    curlTestInfra = curl-test-infra;

    # Allow network access to localhost (test servers bind to 127.0.0.1)
    __darwinAllowLocalNetworking = true;
  } ''
    export TMPDIR="$(mktemp -d)"
    export HOME="$TMPDIR"

    # Set up directory structure expected by runtests.pl:
    # runtests.pl runs from tests/ and expects ../src/curl and ../src/curlinfo
    mkdir -p "$TMPDIR/curl-src"
    cp -r "$curlTestInfra/tests" "$TMPDIR/curl-src/tests"
    cp -r "$curlTestInfra/src" "$TMPDIR/curl-src/src"
    chmod -R u+w "$TMPDIR/curl-src"

    cd "$TMPDIR/curl-src/tests"

    # Make test server libraries available (for curlinfo and test servers)
    export LD_LIBRARY_PATH="$curlTestInfra/lib:''${LD_LIBRARY_PATH:-}"

    echo "Running curl test: ${testNumStr}"

    # Run the test with rust-curl as the binary under test
    # -n = no valgrind
    perl ./runtests.pl \
      -c "${pkgs.rust-curl-dev}/bin/curl" \
      -n \
      ${testNumStr} \
      > "$TMPDIR/output" 2>&1 || {
        cat "$TMPDIR/output"
        exit 1
      }

    cat "$TMPDIR/output"
    touch $out
  ''
