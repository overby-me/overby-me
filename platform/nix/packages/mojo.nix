# Mojo, from the wheels Modular publishes, for x86_64 and aarch64.
#
# This tree built the whole compiler from source with Bazel until the binary
# releases covered both arches; `jj log` has that derivation if a patched
# toolchain is ever worth the hours again. Note that it never did build on
# aarch64 - its hash there was `lib.fakeHash`, a placeholder no build had
# filled in.
#
# Upstream splits one toolchain across four wheels, and the names mislead:
#   mojo-compiler            the compiler. `bin/mojo`, `bin/lld`, runtime libs.
#   mojo-compiler-mojo-libs  the stdlib, as a prebuilt `std.mojoc`. Arch-free.
#   mojo                     NOT the compiler: its own metadata calls it the
#                            development wheel and it declares no `mojo`
#                            console script, only the LSP server, the REPL
#                            entry point and lldb.
#   mojo-lldb-libs           liblldb, which the development wheel's debugger
#                            binaries link but do not contain.
#
# The driver finds everything through `[mojo-max] package_root` in
# modular.cfg, which it reads from MODULAR_HOME. Every key left unset there
# resolves package_root-relative (bin/mojo, bin/lld, lib/*.so), so the wheels'
# `modular/` prefix becomes $out and the config stays four lines.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  unzip,
  libbsd,
  ncurses,
  python3,
  zlib,
  zstd,
}: let
  version = "1.1.0";
  # `mojo format` shells out to mblack, upstream's fork of black. Pure Python,
  # and every dependency it declares is in nixpkgs.
  mblackVersion = "26.6.0";
  mblackEnv = python3.withPackages (ps:
    with ps; [
      click
      mypy-extensions
      pathspec
      platformdirs
    ]);

  wheel = name: file: hash:
    fetchurl {
      inherit hash;
      url = "https://files.pythonhosted.org/packages/${file}";
      name = "${name}.whl";
    };

  # Per-arch halves. The stdlib wheel is py3-none-any and shared.
  byArch = {
    x86_64-linux = {
      compiler = wheel "mojo-compiler-${version}-x86_64" "e8/5e/31522e5336d63ab0fad18980cccf6c5cc2085c1a041ea3723eb916a17c08/mojo_compiler-${version}-py3-none-manylinux_2_34_x86_64.whl" "sha256-tHRCJ46zlwUHXyPQWLOKNkvWi9qlQq1RTBh87mM76rc=";
      tools = wheel "mojo-tools-${version}-x86_64" "dd/c0/4118df04da5776700839642477bdda0b1f906a99c69c8f2f94d4aea9fef4/mojo-${version}-py3-none-manylinux_2_34_x86_64.whl" "sha256-qpU4PZYNLXqHOBF+2AjfdF+l/JLSRWasc+d1qN7+gy0=";
      lldb = wheel "mojo-lldb-libs-${version}-x86_64" "93/f3/abf242879fe2f26809faa351bfb52d0746c25dde78796c3c5b5443355ea5/mojo_lldb_libs-${version}-py3-none-manylinux_2_34_x86_64.whl" "sha256-eC/1ZrFBAGNKcX1bWWAuazEPRIAQfjkQR9sx0U80l6o=";
      ncursesConda = fetchurl {
        url = "https://conda.anaconda.org/conda-forge/linux-64/ncurses-6.6-hdb14827_1.conda";
        hash = "sha256-XUZVchTtGEOB2v6DW3yUpHShw7MHoIolCx6kd5tE/7M=";
      };
    };
    aarch64-linux = {
      compiler = wheel "mojo-compiler-${version}-aarch64" "67/1c/2c93f967615e3581b124b1b5f4af29913994199e8741736d9c08ed3d9f13/mojo_compiler-${version}-py3-none-manylinux_2_34_aarch64.whl" "sha256-FO1/41cOE3Soi8iqk9xOnFGeoo5/jXMa5L8cRo2Beew=";
      tools = wheel "mojo-tools-${version}-aarch64" "22/99/ce31f21811ed70c7499ed4db6780af0c0ea51833ae7214f696977d3f451b/mojo-${version}-py3-none-manylinux_2_34_aarch64.whl" "sha256-MY1GfM3O+crvGnd5J1iAZnK1mIH5D8LoIOcb1Z3gQeI=";
      lldb = wheel "mojo-lldb-libs-${version}-aarch64" "2b/f7/ca628dab5ba5a9c5fc995d671fef1f9344d5684bd041372670a49846fbe2/mojo_lldb_libs-${version}-py3-none-manylinux_2_34_aarch64.whl" "sha256-C9XjY2w0RPLIhYMSfI6dlQABb+RQqXCWqsTuIQgyXOM=";
      ncursesConda = fetchurl {
        url = "https://conda.anaconda.org/conda-forge/linux-aarch64/ncurses-6.6-h2b6f883_1.conda";
        hash = "sha256-1poEkUE5Yn8Ka/0ZQS1sfx437ciWr4SmAR4H6Lweafo=";
      };
    };
  };

  stdlib = wheel "mojo-stdlib-${version}" "83/8d/4fb2147d140b6ea688f6d256b3119d6554a3d50095d802f01bf24234bd7d/mojo_compiler_mojo_libs-${version}-py3-none-any.whl" "sha256-3YxTguJ/5cH/4INNJn9Umrv/7jW/Go1gXPJCUv+VouY=";

  mblack = wheel "mblack-${mblackVersion}" "8d/1e/69f18cd59c58c67224f5c3fa39cc17d1adc0649a064d8806eb7c350bebce/mblack-${mblackVersion}-py3-none-any.whl" "sha256-Tt2kwb19ddA3CSgHd/87veuSiRVoNAApAV/TnyExIOA=";

  arch = byArch.${stdenv.hostPlatform.system};
in
  stdenv.mkDerivation {
    pname = "mojo";
    inherit version;

    srcs = [arch.compiler arch.tools arch.lldb stdlib mblack arch.ncursesConda];

    nativeBuildInputs = [autoPatchelfHook makeWrapper unzip zstd];

    buildInputs = [
      stdenv.cc.cc.lib
      # liblldb and the debug server want libbsd; everything else resolves
      # against the libraries the wheels carry.
      libbsd
      ncurses
      zlib
    ];

    sourceRoot = ".";

    # Wheels and .conda archives are both zip; the conda one nests its payload
    # in a zstd tarball.
    unpackPhase = ''
      runHook preUnpack
      for src in $srcs; do unzip -qo "$src"; done
      for pkg in pkg-*.tar.zst; do
        [ -e "$pkg" ] || continue
        mkdir -p conda && tar --zstd -xf "$pkg" -C conda && rm "$pkg"
      done
      runHook postUnpack
    '';

    installPhase = ''
      runHook preInstall

      # Each wheel stages its payload under <dist>.data/platlib/modular; the
      # three overlay into one toolchain.
      # Modes carry over from the wheels, which mark the binaries executable;
      # only ownership is dropped, and u+w is restored so fixup can patch them.
      mkdir -p $out
      for d in *.data/platlib/modular; do
        cp -r --no-preserve=ownership "$d"/. $out/
      done
      chmod -R u+w $out

      # liblldb wants NCURSES6_5.0.19991023-versioned symbols, which nixpkgs'
      # ncurses does not define and conda-forge's does; without these the
      # debugger dies at load. Placing them in $out/lib is enough, because
      # autoPatchelf resolves a package's own libraries before the inputs.
      # The compiler reaches terminfo through TERMINFO_DIRS instead and is
      # unaffected either way.
      install -Dm755 -t $out/lib \
        conda/lib/libncurses.so.6 conda/lib/libpanel.so.6 \
        conda/lib/libform.so.6 conda/lib/libmenu.so.6

      # mblack is a plain wheel: its modules sit at the archive root rather
      # than under a .data prefix, and it is three of them, not just the
      # package directory.
      mkdir -p $out/share/mblack
      cp -r --no-preserve=ownership mblack mblib2to3 _mblack_version.py \
        $out/share/mblack/
      chmod -R u+w $out/share/mblack
      # nixpkgs' pathspec predates PathSpec.__class_getitem__ and these
      # subscripts are type annotations that CPython evaluates at import
      # time; the bare class means the same thing to a formatter.
      sed -i 's/PathSpec\[PathSpecPattern\]/PathSpec/g' \
        $out/share/mblack/mblack/__init__.py $out/share/mblack/mblack/files.py
      # The package declares its console script as mblack:patched_main and
      # ships no __main__, so -m cannot run it; this is that shim. Running a
      # file rather than -c also keeps argv[0] out of --version output.
      cat > $out/share/mblack/__main.py <<'PY'
      import sys

      from mblack import patched_main

      # click names itself after argv[0], which is this shim's path.
      sys.argv[0] = "mblack"
      sys.exit(patched_main())
      PY

      # PYTHONSAFEPATH keeps the working directory off sys.path, where a
      # checkout's own mblack/ would otherwise shadow this one.
      makeWrapper ${mblackEnv}/bin/python $out/bin/mblack \
        --prefix PYTHONPATH : $out/share/mblack \
        --set PYTHONSAFEPATH 1 \
        --add-flags "$out/share/mblack/__main.py"

      mkdir -p $out/etc/modular
      cat > $out/etc/modular/modular.cfg <<EOF
      [max]
      name = Mojo
      version = ${version}

      [mojo-max]
      package_root = $out
      import_path = $out/lib/mojo
      lldb_path = $out/bin/mojo-lldb
      mblack_path = $out/bin/mblack
      system_libs = -lrt,-ldl,-lpthread,-lm,-lz,-ltinfo
      EOF

      # The driver derives its cache and crash directories from MODULAR_HOME,
      # which must therefore be writable; give each user their own under XDG
      # and seed it with the store config.
      modularHome='"''${MODULAR_HOME:-''${XDG_CACHE_HOME:-$HOME/.cache}/mojo}"'
      for tool in mojo mojo-lldb; do
        mv $out/bin/$tool $out/bin/$tool-unwrapped
        makeWrapper $out/bin/$tool-unwrapped $out/bin/$tool \
          --run "export MODULAR_HOME=$modularHome" \
          --run 'mkdir -p "$MODULAR_HOME"' \
          --run "ln -sfn $out/etc/modular/modular.cfg \"\$MODULAR_HOME/modular.cfg\"" \
          --set-default MODULAR_CRASH_REPORTING_ENABLED 0 \
          --set-default MODULAR_TELEMETRY_ENABLED 0 \
          --set-default TERMINFO_DIRS ${ncurses}/share/terminfo
      done

      # The language server resolves imports itself rather than through the
      # driver, so it needs the stdlib named on its own command line.
      mv $out/bin/mojo-lsp-server $out/bin/mojo-lsp-server-unwrapped
      makeWrapper $out/bin/mojo-lsp-server-unwrapped $out/bin/mojo-lsp-server \
        --run "export MODULAR_HOME=$modularHome" \
        --run 'mkdir -p "$MODULAR_HOME"' \
        --run "ln -sfn $out/etc/modular/modular.cfg \"\$MODULAR_HOME/modular.cfg\"" \
        --add-flags "-I $out/lib/mojo"

      runHook postInstall
    '';

    doInstallCheck = true;
    installCheckPhase = ''
      runHook preInstallCheck

      export HOME="$(mktemp -d)"
      $out/bin/mojo --version
      $out/bin/mojo-lsp-server --version
      # Loads liblldb, so this is what catches the ncurses symbol mismatch.
      $out/bin/mojo-lldb --version
      $out/bin/mblack --version

      # Compile and run, so the check covers the stdlib and the linker rather
      # than just the driver starting up.
      cd "$(mktemp -d)"
      {
        echo 'def main():'
        echo '    print("mojo runs")'
      } > smoke.mojo
      $out/bin/mojo build smoke.mojo -o smoke
      ./smoke | grep -q "mojo runs"

      runHook postInstallCheck
    '';

    meta = {
      description = "Mojo programming language toolchain, from Modular's release wheels";
      homepage = "https://www.modular.com/mojo";
      downloadPage = "https://pypi.org/project/mojo-compiler/";
      # Apache-2.0 with LLVM exceptions on the Python and stdlib sources that
      # carry a header; the compiler binary itself ships no license grant.
      license = lib.licenses.unfree;
      sourceProvenance = with lib.sourceTypes; [binaryNativeCode];
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.attrNames byArch;
      mainProgram = "mojo";
    };
  }
