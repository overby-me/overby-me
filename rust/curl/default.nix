{
  packages = {
    rust-curl = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-curl";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        meta = {
          description = "A curl-compatible HTTP client written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/curl";
          license = lib.licenses.mit;
          mainProgram = "curl";
        };
      };

    rust-curl-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-curl-dev";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        buildType = "debug";

        meta = {
          description = "A curl-compatible HTTP client written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/curl";
          license = lib.licenses.mit;
          mainProgram = "curl";
        };
      };
  };

  # Discovery tool: runs tests 1-100 and reports which pass/fail
  # Usage: nix build .#packages.x86_64-linux.rust-curl-test-discovery -L
  packages.rust-curl-test-discovery = {
    lib,
    perl,
    coreutils,
    diffutils,
    gnused,
    gnugrep,
    stunnel,
    rust-curl-dev,
    curl,
    stdenv,
    autoreconfHook,
    pkg-config,
    python3,
    openssl,
    zlib,
    nghttp2,
    libpsl,
  }: let
    curl-test-infra = stdenv.mkDerivation {
      pname = "curl-test-infra";
      inherit (curl) version src;
      nativeBuildInputs = [autoreconfHook pkg-config perl python3];
      buildInputs = [openssl zlib nghttp2 libpsl];
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
      buildPhase = ''
        make -C lib -j$NIX_BUILD_CORES
        make -C src -j$NIX_BUILD_CORES
        make -C tests -j$NIX_BUILD_CORES
      '';
      installPhase = ''
        mkdir -p $out/lib $out/src
        cp -r tests $out/tests
        cp src/.libs/curl $out/src/curl 2>/dev/null || cp src/curl $out/src/curl
        if [ -f src/.libs/curlinfo ]; then cp src/.libs/curlinfo $out/src/curlinfo;
        elif [ -f src/curlinfo ]; then cp src/curlinfo $out/src/curlinfo; fi
        cp lib/.libs/libcurl.so* $out/lib/ 2>/dev/null || true
        chmod +x $out/tests/runtests.pl
      '';
      dontStrip = true;
    };
  in
    lib.warn "This derivation runs tests 1-200 in batch; use -L to see live output"
    (derivation {
      name = "rust-curl-test-discovery";
      inherit (stdenv) system;
      builder = "${stdenv.shell}";
      args = [
        "-c"
        ''
          export PATH="${lib.makeBinPath [perl coreutils diffutils gnused gnugrep stunnel rust-curl-dev]}"
          export TMPDIR=$(${coreutils}/bin/mktemp -d)
          export HOME="$TMPDIR"

          ${coreutils}/bin/cp -r "${curl-test-infra}/tests" "$TMPDIR/tests"
          ${coreutils}/bin/cp -r "${curl-test-infra}/src" "$TMPDIR/src"
          ${coreutils}/bin/chmod -R u+w "$TMPDIR/tests" "$TMPDIR/src"
          cd "$TMPDIR/tests"
          export LD_LIBRARY_PATH="${curl-test-infra}/lib"

          ${perl}/bin/perl ./runtests.pl \
            -c "${rust-curl-dev}/bin/curl" \
            -n \
            -a \
            1 to 200 \
            2>&1 | ${coreutils}/bin/tee "$TMPDIR/results.txt" || true

          ${coreutils}/bin/mkdir -p $out
          ${coreutils}/bin/cp "$TMPDIR/results.txt" $out/results.txt
        ''
      ];
      __darwinAllowLocalNetworking = true;
    });

  checks = let
    testNums = [
      1 # HTTP GET
      2 # HTTP GET with user and password
      3 # HTTP POST with auth
      7 # HTTP with cookie parser and header recording
      10 # simple HTTP PUT from file
      11 # simple HTTP Location following
      13 # HTTP GET with -i
      15 # --write-out test
      22 # HTTP PUT with upload file
      28 # HTTP with -D dump-header
      34 # HTTP with --head
      35 # HTTP GET with custom header
      47 # HTTP GET with custom Host
      49 # HTTP with Expect: 100-continue
      50 # HTTP follow redirect with ../../
      52 # HTTP follow redirect with abs path
      55 # HTTP follow redirect with relative path
      57 # HTTP GET with added header
      97 # HTTP with -i response headers
      152 # HTTP GET with content-range
      160 # HTTP GET simple
    ];
  in
    builtins.listToAttrs (map (num: {
        name = "rust-curl-test-${toString num}";
        value = pkgs:
          import ./testsuite.nix {
            inherit pkgs;
            testNum = num;
          };
      })
      testNums);
}
