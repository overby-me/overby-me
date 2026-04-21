# Helpers that wire up upstream xz tests (extracted from `pkgs.xz.src`)
# against the locally-built `rust-xz-dev` binary, expressed as Nix
# checks so they can be enumerated by `nix flake check`.
#
# Mirrors the structure of `rust/awk/testsuite.nix`.
{pkgs}: rec {
  # Run one of the upstream `tests/test_*.sh` scripts.
  #
  # Calling conventions vary per script:
  # * `test_files.sh`, `test_suffix.sh`, `test_scripts.sh` — `$1 = dir
  #   containing the xz binary` (looked up as `${1}/xz`).
  # * `test_compress.sh` — `$1 = test filename` (a `compress_generated_*`
  #   or `compress_prepared_*` token), `$2 = dir containing xz`. We
  #   default `$1` to `compress_prepared_bcj_sparc` (a fixture file
  #   already shipped under `tests/files/`) so the script does not
  #   need a `create_compress_files` helper.
  # * The `test_compress_generated_*` wrappers exec `test_compress.sh`
  #   with a `compress_generated_*` filename; running them requires
  #   `create_compress_files`, so we build it via xz's own makefile.
  #
  # `allowSkip` (default false) determines what to do with an exit-77
  # ("skipped") return: by default we treat a skip as a failure so an
  # accidental misconfiguration (wrong $1, missing dependency) cannot
  # silently flag the check as green. Set `allowSkip = true` only when
  # the skip reason is genuinely out of scope (e.g. `test_scripts.sh`
  # needs `xzdiff`/`xzgrep`).
  script = {
    name,
    allowSkip ? false,
    compressFile ? "compress_prepared_bcj_sparc",
    needsHelper ? false,
  }:
    pkgs.runCommand "rust-xz-test-${name}" {
      nativeBuildInputs =
        [pkgs.rust-xz-dev pkgs.gnutar pkgs.coreutils pkgs.xz]
        ++ pkgs.lib.optionals needsHelper [pkgs.gcc pkgs.gnumake pkgs.autoconf pkgs.automake pkgs.libtool pkgs.gettext pkgs.pkg-config pkgs.m4 pkgs.po4a];
      xzSrc = pkgs.xz.src;
    } ''
      tar xf "$xzSrc"
      cd xz-*/tests

      mkdir -p fake
      ln -s ${pkgs.rust-xz-dev}/bin/xz fake/xz

      ${pkgs.lib.optionalString needsHelper ''
        # Build `create_compress_files` from the upstream tarball.
        # The C source includes sysdefs.h which expects a config.h to
        # exist, so we must run autoreconf+configure before make. Using
        # `make check_PROGRAMS=create_compress_files` builds only the
        # one helper we need, skipping the rest of liblzma.
        pushd ..
        export ACLOCAL_PATH="${pkgs.gettext}/share/aclocal:$ACLOCAL_PATH"
        autoreconf -fi >/dev/null 2>&1 || true
        ./configure --disable-doc --disable-nls --disable-shared \
                    --enable-static --quiet >/dev/null
        make -C src/liblzma >/dev/null
        make -C tests create_compress_files >/dev/null
        popd
      ''}

      export srcdir=.
      export TMPDIR=$(mktemp -d)

      # Script filenames: most upstream tests are `<name>.sh`,
      # but the `test_compress_generated_*` wrappers don't have an
      # extension. Detect the right path at runtime.
      run_script() {
        echo "Running upstream test: ${name}"
        ${
          if name == "test_compress" || (pkgs.lib.hasPrefix "test_compress_generated_" name)
          # The upstream `test_compress_generated_*` wrappers exec
          # `test_compress.sh <filename>` with no xz-dir argument,
          # which makes the script default to `../src/xz` and skip.
          # We bypass them and call `test_compress.sh` directly with
          # both arguments so the rust-xz binary is actually used.
          then ''sh ./test_compress.sh "${compressFile}" "./fake"''
          else ''sh "./${name}.sh" "./fake"''
        }
      }

      run_script || {
        rc=$?
        if [ $rc -eq 77 ]; then
          ${
            if allowSkip
            then ''
              echo "test ${name}.sh skipped (exit 77, allowSkip=true)"
              touch $out
              exit 0
            ''
            else ''
              echo "test ${name}.sh exited 77 (skipped) but allowSkip is false"
              echo "this almost always means the test was misconfigured."
              exit 1
            ''
          }
        fi
        echo "test ${name}.sh failed (exit $rc)"
        exit $rc
      }
      touch $out
    '';

  # Decode a single sample file from `tests/files/` and assert the
  # outcome. `expect` is either "good" (must succeed) or "bad" (must
  # fail). `name` is the leaf filename inside `tests/files/`.
  file = {
    name,
    expect,
  }:
    pkgs.runCommand "rust-xz-decode-${name}" {
      nativeBuildInputs = [pkgs.rust-xz-dev pkgs.gnutar];
      xzSrc = pkgs.xz.src;
    } ''
      tar xf "$xzSrc"
      F=$(echo xz-*/tests/files/${name})
      if [ ! -f "$F" ]; then
        echo "missing input file: $F"
        exit 1
      fi

      if ${pkgs.rust-xz-dev}/bin/xz -dc "$F" > /dev/null 2>&1; then
        case "${expect}" in
          good) touch $out ;;
          bad)  echo "bad input was accepted: ${name}"; exit 1 ;;
        esac
      else
        case "${expect}" in
          good) echo "good input was rejected: ${name}"; exit 1 ;;
          bad)  touch $out ;;
        esac
      fi
    '';

  # Quick standalone round-trip sanity check: compress + decompress a
  # known buffer at every preset level using the rust-xz binary.
  roundtrip = pkgs.runCommand "rust-xz-roundtrip" {
    nativeBuildInputs = [pkgs.rust-xz-dev pkgs.coreutils pkgs.diffutils];
  } ''
    set -e
    TMP=$(mktemp -d)
    head -c 65536 /dev/urandom > "$TMP/payload"
    for level in 0 1 2 3 4 5 6 7 8 9; do
      ${pkgs.rust-xz-dev}/bin/xz -c -$level "$TMP/payload" > "$TMP/out.xz"
      ${pkgs.rust-xz-dev}/bin/xz -dc "$TMP/out.xz" > "$TMP/decoded"
      if ! cmp "$TMP/payload" "$TMP/decoded"; then
        echo "roundtrip mismatch at level $level"
        exit 1
      fi
    done
    touch $out
  '';

  # End-to-end smoke check for `xz -l`/`--list`. Compresses a file,
  # then asserts the list output contains the expected stream count
  # and the CRC64 check name (xz's default).
  list = pkgs.runCommand "rust-xz-list" {
    nativeBuildInputs = [pkgs.rust-xz-dev pkgs.coreutils pkgs.gnugrep];
  } ''
    set -e
    TMP=$(mktemp -d)
    printf 'list mode integration check\n' > "$TMP/payload"
    ${pkgs.rust-xz-dev}/bin/xz -k -c "$TMP/payload" > "$TMP/payload.xz"
    OUT=$(${pkgs.rust-xz-dev}/bin/xz -l "$TMP/payload.xz")
    echo "$OUT"
    echo "$OUT" | grep -q "Strms"
    echo "$OUT" | grep -q "CRC64"
    echo "$OUT" | grep -q "payload.xz"
    touch $out
  '';

  # End-to-end smoke check for the BCJ filter chain (`--x86`,
  # `--arm64`, `--filters=`). Round-trips a payload through each
  # combination and asserts byte-equality.
  filters = pkgs.runCommand "rust-xz-filters" {
    nativeBuildInputs = [pkgs.rust-xz-dev pkgs.coreutils pkgs.diffutils];
  } ''
    set -e
    TMP=$(mktemp -d)
    head -c 16384 /dev/urandom > "$TMP/p"

    # --x86 + --lzma2= short-flag form.
    ${pkgs.rust-xz-dev}/bin/xz --x86 --lzma2=preset=4 -c "$TMP/p" > "$TMP/p.xz"
    ${pkgs.rust-xz-dev}/bin/xz -dc "$TMP/p.xz" > "$TMP/p2"
    cmp "$TMP/p" "$TMP/p2"

    # --filters= form with a different BCJ + LZMA2 chain.
    ${pkgs.rust-xz-dev}/bin/xz --filters="arm64 lzma2:preset=4" -c "$TMP/p" > "$TMP/p.xz"
    ${pkgs.rust-xz-dev}/bin/xz -dc "$TMP/p.xz" > "$TMP/p2"
    cmp "$TMP/p" "$TMP/p2"

    # --filters= using the `--` token separator (same as upstream).
    ${pkgs.rust-xz-dev}/bin/xz --filters="riscv--lzma2:preset=4" -c "$TMP/p" > "$TMP/p.xz"
    ${pkgs.rust-xz-dev}/bin/xz -dc "$TMP/p.xz" > "$TMP/p2"
    cmp "$TMP/p" "$TMP/p2"

    touch $out
  '';

  # Decoder-stability fuzz check. Runs the `rust-xz-fuzz` binary
  # (built as part of `rust-xz-dev`) against the upstream
  # `tests/files/` corpus. Every file in the corpus is fed to the
  # decoder verbatim, then with prefix-truncated and one-byte-flipped
  # mutations; the test passes iff the decoder never panics.
  fuzz = pkgs.runCommand "rust-xz-fuzz" {
    nativeBuildInputs = [pkgs.rust-xz-dev pkgs.gnutar pkgs.coreutils];
    xzSrc = pkgs.xz.src;
  } ''
    set -e
    tar xf "$xzSrc"
    CORPUS=$(echo "$PWD"/xz-*/tests/files)
    echo "fuzz corpus: $CORPUS ($(ls "$CORPUS" | wc -l) files)"
    ${pkgs.rust-xz-dev}/bin/rust-xz-fuzz "$CORPUS"
    touch $out
  '';

  # Run one of the upstream `tests/test_*.c` C unit tests. These
  # exercise liblzma library internals (VLI codec, block header
  # parser, index hash, filter-flags codec, lzip decoder, etc.) —
  # they don't touch the `xz` CLI surface we wrap, but they
  # validate the C library that `rust-xz` links against (via the
  # `liblzma` crate) so they're useful as upstream-parity coverage.
  #
  # We re-use upstream's own `Makefile` target and run a single
  # binary per Nix derivation so failures are isolated.
  cTest = name:
    pkgs.runCommand "rust-xz-${name}" {
      nativeBuildInputs = [
        pkgs.gcc pkgs.gnumake pkgs.autoconf pkgs.automake pkgs.libtool
        pkgs.gettext pkgs.pkg-config pkgs.m4 pkgs.po4a pkgs.gnutar
      ];
      xzSrc = pkgs.xz.src;
    } ''
      tar xf "$xzSrc"
      cd xz-*

      autoreconf -fi >/dev/null 2>&1 || true
      ./configure --disable-doc --disable-nls --disable-shared \
                  --enable-static --quiet >/dev/null
      make -C src/liblzma >/dev/null
      make -C tests "${name}" >/dev/null

      cd tests
      export srcdir=.
      ./${name} || {
        rc=$?
        if [ $rc -eq 77 ]; then
          echo "C test ${name} self-skipped (exit 77)"
          touch $out
          exit 0
        fi
        echo "C test ${name} failed (exit $rc)"
        exit $rc
      }
      touch $out
    '';
}
