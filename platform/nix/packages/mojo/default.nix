# Mojo, built from source.
#
# Modular open-sourced the Mojo compiler on 2026-08-18 (Apache 2.0 with LLVM
# exceptions), so this package builds the whole toolchain from
# github.com/modular/modular instead of repackaging the proprietary conda
# binaries it used through 26.2.0.
#
# Shape of the build:
#
#   - Upstream builds with Bazel (`./bazelw build --config=build-mojo
#     //KGEN:mojo`). We drive nixpkgs' bazel_9 in three derivations: a
#     fixed-output `deps` derivation runs `bazel build --nobuild` with
#     `--repository_cache` and keeps only that cache, i.e. the pristine
#     content-addressed downloads; a heavy `build` derivation replays the
#     same build offline against it and exports the raw products (the
#     binaries plus every shared library bazel produced, so no library can
#     turn out to be missing later); the cheap top-level derivation does
#     all the packaging - layout, patchelf, wrappers, config, smoke tests -
#     so packaging mistakes cost seconds to fix instead of a multi-hour
#     bazel rebuild.
#
#   - Bazel executes downloaded tools while it fetches repositories
#     (rules_python runs its hermetic CPython, the build itself runs
#     Modular's prebuilt clang 22), and none of them start inside the
#     sandbox with their /lib64 interpreters. Both bazel derivations
#     therefore run bazel in a retry loop: on failure, every ELF in bazel's
#     external/ tree is patchelfed to the nix dynamic linker and the build
#     resumes. The FOD's output stays byte-pristine because the cache
#     holds unextracted archives; the patched files live only in the
#     discarded output base.
#
#   - LLVM cannot come from nixpkgs: the compiler pins llvm-project at a
#     trunk commit (see bazel/public-patches/llvm_source.bzl) and applies
#     six Modular patches on top, including a 16K lldb-exports patch that
#     mojo-lldb needs. No released LLVM has that API surface, so Bazel
#     builds the patched snapshot from source as upstream intends. nixpkgs
#     does supply bazel, the Python for mblack, and the runtime libraries
#     (zlib, ncurses, libedit) that used to come from conda. The patch
#     also adds the WebAssembly backend, which upstream leaves out but
#     dev/mojo/gui's wasm builds need; the conda compiler carried every
#     backend. llc and lld (as wasm-ld) ship with the SDK for the same
#     reason: they must parse the IR this exact LLVM emits.
#
#   - `--//:modular_config=production -c opt` matches the shipped release
#     binaries (-O3, -DNDEBUG); the repo's developer default is a debug
#     build.
#
# The installed layout keeps the contract of the old package (bin/mojo,
# bin/mojo-lsp-server, bin/mojo-lldb, bin/mblack, lib/mojo import path,
# etc/modular/modular.cfg + MODULAR_HOME wrappers): the driver resolves every
# tool through `mojo-max.package_root` relative defaults, so the cfg only
# names the few keys without one. `mojo build` links executables with the
# `cc` found on PATH, which is why consumers keep working by adding their
# link libraries to buildInputs.
{
  lib,
  stdenv,
  fetchFromGitHub,
  bazel_9,
  python3,
  autoPatchelfHook,
  makeWrapper,
  patchelf,
  coreutils,
  util-linux,
  lndir,
  zlib,
  ncurses,
  libedit,
  libxml2_13,
  libbsd,
  expat,
}: let
  # Nightly-versioned main; the mojo/v1.0.0 tag (2026-08-11) predates the
  # compiler drop and has no KGEN directory.
  version = "1.1.0-dev2026081813";
  rev = "f66d4d522c34be0a961ffac3dbfc81e30f67942e";

  src = fetchFromGitHub {
    owner = "modular";
    repo = "modular";
    inherit rev;
    hash = "sha256-ieecVlQ6nyFyb3LebwhMTtCP6y9FCbVEkTIrNj63hbM=";
  };

  # The FHS dynamic-linker path this platform's ELF binaries hardcode in
  # PT_INTERP; the build namespace recreates exactly this path so the
  # prebuilt tools bazel downloads can start inside the sandbox.
  fhsInterp =
    {
      x86_64-linux = "/lib64/ld-linux-x86-64.so.2";
      aarch64-linux = "/lib/ld-linux-aarch64.so.1";
    }
    .${
      stdenv.hostPlatform.system
    };

  mblackPythonEnv = python3.withPackages (ps:
    with ps; [
      click
      mypy-extensions
      pathspec
      platformdirs
      tomli
    ]);

  # Identical flags for the fetch and the build so both analyze the same
  # graph and the repository cache is complete.
  bazelFlags = lib.concatStringsSep " " [
    "--config=build-mojo"
    "--//:modular_config=production"
    "--compilation_mode=opt"
    "--//:modular_version_sha=${rev}"
    # The nix sandbox is the isolation boundary; bazel's own sandbox cannot
    # use user namespaces in here and would fall back anyway, slowly. Output
    # path mapping (a remote-cache-key optimization from the repo's bazelrc)
    # requires a sandboxed strategy, so switch it off alongside.
    "--spawn_strategy=local"
    "--experimental_output_paths=off"
    # Freshly built tools (tblgen, then mojo itself compiling the stdlib)
    # run as later build actions, so they must be executable inside the
    # sandbox from the moment they are linked: give every link the nix
    # dynamic linker and the runtime-library rpath up front. Both
    # configurations, because tools build in the exec config.
    "--linkopt=-Wl,--dynamic-linker=${stdenv.cc.bintools.dynamicLinker}"
    "--host_linkopt=-Wl,--dynamic-linker=${stdenv.cc.bintools.dynamicLinker}"
    "--linkopt=-Wl,-rpath,${lib.makeLibraryPath runtimeDeps}"
    "--host_linkopt=-Wl,-rpath,${lib.makeLibraryPath runtimeDeps}"
    # Prebuilt tools (clang's ld.lld wants libxml2.so.2 and libz) resolve
    # their libraries through the action environment; the namespace only
    # restores the loader, not a populated /usr/lib.
    "--action_env=LD_LIBRARY_PATH=${lib.makeLibraryPath runtimeDeps}"
    "--host_action_env=LD_LIBRARY_PATH=${lib.makeLibraryPath runtimeDeps}"
    # Large MLIR translation units: leave headroom so parallel clang + lld
    # invocations do not OOM the builder.
    "--local_resources=memory=HOST_RAM*.67"
  ];

  # The fetch set: everything `deps` pre-downloads. Kept separate from the
  # build set below so adding targets that need no new repositories does
  # not invalidate the fixed-output hash.
  sdkTargets = lib.concatStringsSep " " [
    "//KGEN:mojo" # mojo-full: driver, with MojoLLDB, lldb, lldb-server and the repl entry point as data
    "//KGEN:CompilerRT" # libKGENCompilerRTShared.so, dlopened for JIT/REPL
    "//KGEN:MojoJupyter" # libMojoJupyter.so, loaded by the jupyter kernel
    "//KGEN/tools/mojo-lsp-server"
    "//KGEN/tools/mojo-repl-entry-point"
    "@mojo//:std" # standard library, std.mojoc
    "@llvm-project//lld:lld"
    "@llvm-project//llvm:llvm-symbolizer"
  ];

  # llc compiles the LLVM IR that `mojo build --emit llvm` produces; only
  # this pinned trunk LLVM is guaranteed to parse its own output, so ship
  # it rather than leaning on a nixpkgs release. Same repositories as the
  # fetch set, so `deps` stays valid.
  buildTargets = sdkTargets + " @llvm-project//llvm:llc";

  # Runtime libraries the patched tools and the installed SDK link against.
  # libxml2_13: Modular's prebuilt ld.lld wants the pre-2.14 libxml2.so.2
  # SONAME, which current libxml2 (.so.16) no longer provides.
  runtimeDeps = [
    stdenv.cc.cc.lib
    zlib
    ncurses
    libedit
    libxml2_13
    expat
  ];

  # Run bazel until it sticks: after every failure, make the ELF tools that
  # repository rules downloaded this round runnable in the sandbox (nix
  # dynamic linker, rpath into nixpkgs and back into their own repo), then
  # retry. Completed repositories and actions are reused, so each round
  # only redoes the step that failed.
  bazelWithRetries = command: ''
    export HOME="$NIX_BUILD_TOP/bazel-home"
    mkdir -p "$HOME"
    export USE_BAZEL_VERSION=${bazel_9.version}

    # The sandbox has no /usr/bin/env and no /lib64 loader, but generated
    # tool stubs and freshly linked binaries expect both. User namespaces
    # are available in the nix sandbox, so run bazel inside an
    # overlayfs-chrooted view of / that adds the two paths. Falls back to
    # a plain invocation (leaving the patchelf sweeps to cope) if the
    # namespace setup fails.
    runBazel() {
      export BAZEL_NS_CMD="$*"
      if unshare -rm /bin/sh -e -c '
        m="$NIX_BUILD_TOP/nsroot"
        mkdir -p "$m"
        mount -t tmpfs none "$m" || exit 97
        for p in /* ; do
          d="''${p#/}"
          [ "$d" = "proc" ] && continue
          if [ -d "$p" ]; then
            mkdir -p "$m/$d"
            mount --rbind "$p" "$m/$d" || exit 97
          elif [ -e "$p" ]; then
            touch "$m/$d"
            mount --bind "$p" "$m/$d" || exit 97
          fi
        done
        mkdir -p "$m/proc"
        mount -t proc proc "$m/proc" || mount --rbind /proc "$m/proc" || exit 97
        mkdir -p "$m/usr/bin" "$m${lib.dirOf fhsInterp}"
        ln -sf ${coreutils}/bin/env "$m/usr/bin/env"
        # Stage-1 bootstrap interpreter for rules_python tool stubs; they
        # re-exec the hermetic toolchain interpreter themselves.
        ln -sf ${python3}/bin/python3 "$m/usr/bin/python3"
        ln -sf ${stdenv.cc.bintools.dynamicLinker} "$m${fhsInterp}"
        exec chroot "$m" /bin/sh -c "cd \"$PWD\" && exec $BAZEL_NS_CMD"
      '; then
        return 0
      else
        rc=$?
        if [ "$rc" -eq 97 ]; then
          echo "namespace setup failed; running bazel without it" >&2
          eval "$BAZEL_NS_CMD"
        else
          return "$rc"
        fi
      fi
    }

    sweepExternal() {
      local root f patched=0 failed=0
      for root in "$HOME"/.cache/bazel/_bazel_*/*; do
        [ -d "$root/external" ] || continue
        # Archives extract with read-only directories; patchelf rewrites
        # need writable parents. -L: repositories can be symlink forests
        # into bazel's caches.
        chmod -R u+w "$root/external" 2>/dev/null || true
        # Downloaded tools: point their ELF interpreter and rpath at nix.
        while IFS= read -r -d "" f; do
          [ "$(head -c4 "$f" 2>/dev/null)" = "$(printf '\x7fELF')" ] || continue
          if patchelf --print-interpreter "$f" >/dev/null 2>&1; then
            if patchelf --set-interpreter "$(cat $NIX_CC/nix-support/dynamic-linker)" "$f"; then
              patched=$((patched + 1))
            else
              failed=$((failed + 1))
              echo "sweep: failed to set interpreter on $f" >&2
            fi
          fi
          patchelf --set-rpath '$ORIGIN:$ORIGIN/../lib:${lib.makeLibraryPath runtimeDeps}' "$f" 2>/dev/null || true
        done < <(find -L "$root/external" -type f \( -perm -0100 -o -name '*.so' -o -name '*.so.*' \) -print0 2>/dev/null)
        # Generated wrapper scripts (py_binary stubs and the like) point at
        # /usr/bin/env, which the sandbox does not have.
        while IFS= read -r -d "" f; do
          chmod u+w "$f" 2>/dev/null || true
          sed -i '1s|^#!/usr/bin/env |#!${coreutils}/bin/env |' "$f" 2>/dev/null || true
        done < <(grep -rlZ '^#!/usr/bin/env ' "$root/external" "$root/execroot" 2>/dev/null || true)
      done
      echo "sweep: $patched interpreters set, $failed failures" >&2
      # Self-test the tools the build spawns next, so a still-broken one is
      # named in the log instead of surfacing as a bare exec failure.
      local tool
      for tool in \
        "$HOME"/.cache/bazel/_bazel_*/*/external/+http_archive+clang-linux-*/bin/clang++ \
        "$HOME"/.cache/bazel/_bazel_*/*/external/+http_archive+clang-linux-*/bin/ld.lld \
        "$HOME"/.cache/bazel/_bazel_*/*/external/+http_archive+clang-linux-*/bin/llvm-ar \
        "$HOME"/.cache/bazel/_bazel_*/*/external/rules_python++python+python_3_12_*-unknown-linux-gnu/bin/python3; do
        [ -e "$tool" ] || continue
        if "$tool" --version >/dev/null 2>&1; then
          echo "sweep: ok $tool" >&2
        else
          echo "sweep: STILL BROKEN $tool" >&2
        fi
      done
    }

    tries=0
    until runBazel bazel --batch ${command}; do
      tries=$((tries + 1))
      if [ "$tries" -gt 8 ]; then
        echo "bazel still failing after $tries attempts" >&2
        exit 1
      fi
      echo "bazel attempt $tries failed; patching fetched tools and retrying" >&2
      sweepExternal
    done
  '';

  deps = stdenv.mkDerivation {
    pname = "mojo-deps";
    inherit version src;
    patches = [./nix-build.patch];

    # Actions exec these scripts directly; their #!/bin/bash does not
    # resolve inside the sandbox.
    postPatch = ''
      patchShebangs --build bazel tools utils
    '';

    nativeBuildInputs = [bazel_9 patchelf util-linux];

    buildPhase = ''
      runHook preBuild
      mkdir -p "$NIX_BUILD_TOP/repo-cache"
      ${bazelWithRetries ''build --nobuild --repository_cache="$NIX_BUILD_TOP/repo-cache" --repo_contents_cache= ${bazelFlags} ${sdkTargets}''}
      runHook postBuild
    '';

    installPhase = ''
      mkdir -p $out/repo-cache
      cp -r --reflink=auto "$NIX_BUILD_TOP"/repo-cache/* $out/repo-cache/
      # The lockfile pins every registry file by hash, which is what lets
      # the offline build resolve modules from the cache alone.
      cp MODULE.bazel.lock $out/
    '';

    dontFixup = true;
    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    # Content-addressed per platform: bazel fetches host-arch prebuilt
    # tools (clang, hermetic CPython), so each system pins its own hash.
    # Bringing up a new platform: build once (the fake hash fails with
    # the real one printed), paste it here, build again.
    outputHash =
      {
        x86_64-linux = "sha256-Kgmvo2pyelMsC4SjsxPJjZUS4xE57LOpG/NtOLTSvdc=";
        aarch64-linux = lib.fakeHash;
      }
      .${
        stdenv.hostPlatform.system
      };
  };

  # The multi-hour bazel run, exporting raw products only. Nothing in here
  # may fail after bazel succeeds, and nothing that belongs to packaging
  # (patchelf, wrappers, checks) happens here: those live in the cheap
  # top-level derivation so they can be iterated without rebuilding LLVM.
  build = stdenv.mkDerivation {
    pname = "mojo-bazel-build";
    inherit version src;
    patches = [./nix-build.patch];

    postPatch = ''
      patchShebangs --build bazel tools utils
    '';

    nativeBuildInputs = [bazel_9 patchelf util-linux lndir];

    buildPhase = ''
      runHook preBuild
      # The cache must be writable even though every entry is already
      # present.
      mkdir -p repo-cache
      lndir -silent ${deps}/repo-cache repo-cache
      install -m644 ${deps}/MODULE.bazel.lock MODULE.bazel.lock
      ${bazelWithRetries ''build --repository_cache="$PWD/repo-cache" --repo_contents_cache= ${bazelFlags} ${buildTargets}''}
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p $out/bin $out/lib/mojo $out/share

      llvmBin=bazel-bin/external/+llvm_configure+llvm-project

      install -m755 bazel-bin/KGEN/tools/mojo/mojo-full $out/bin/
      install -m755 bazel-bin/KGEN/tools/mojo-lsp-server/mojo-lsp-server $out/bin/
      install -m755 bazel-bin/KGEN/tools/mojo-repl-entry-point/mojo-repl-entry-point $out/bin/
      install -m755 $llvmBin/lldb/lldb $llvmBin/lldb/lldb-server \
        $llvmBin/lld/lld $llvmBin/llvm/llvm-symbolizer $llvmBin/llvm/llc \
        $out/bin/
      install -m644 bazel-bin/mojo/stdlib/std/std.mojoc $out/lib/mojo/std.mojoc

      # Every shared library bazel produced in the target configuration,
      # deduplicated by basename. The binaries above resolve their NEEDED
      # entries against this pool in the packaging derivation, so a
      # library it turns out to need is already here instead of being a
      # multi-hour rebuild away (libMSupportGlobals.so taught us that).
      while IFS= read -r -d "" so; do
        cp -Ln "$so" $out/lib/ 2>/dev/null || true
      done < <(find -L bazel-bin/ -type f \( -name '*.so' -o -name '*.so.*' \) \
        -not -path '*/_objs/*' -print0 2>/dev/null)

      # Source-tree pieces the packaging derivation re-homes: the jupyter
      # kernel spec and mblack, which are plain Python.
      cp -r KGEN/utils/jupyter-mojo/kernel $out/share/jupyter-kernel
      cp -r KGEN/tools/mblack/src $out/share/mblack-src

      runHook postInstall
    '';

    # Raw export: no strip (std.mojoc and the .so pool must stay
    # byte-exact), no patchelf, no shebang rewrites.
    dontFixup = true;
  };
in
  stdenv.mkDerivation {
    pname = "mojo";
    inherit version;

    dontUnpack = true;

    nativeBuildInputs = [
      autoPatchelfHook
      makeWrapper
    ];
    # libbsd: lldb-server links libbsd.so.0.
    buildInputs = runtimeDeps ++ [libbsd];
    # Optional diagnostics libraries the lldb build references but the SDK
    # runs fine without; installCheckPhase is the arbiter of "works".
    autoPatchelfIgnoreMissingDeps = true;
    dontStrip = true;

    installPhase = ''
      runHook preInstall

      mkdir -p $out/bin $out/lib/mojo $out/etc/modular

      install -m755 ${build}/bin/mojo-full $out/bin/mojo-unwrapped
      install -m755 ${build}/bin/mojo-lsp-server $out/bin/mojo-lsp-server-unwrapped
      install -m755 ${build}/bin/mojo-repl-entry-point $out/lib/mojo-repl-entry-point
      install -m755 ${build}/bin/lldb $out/bin/mojo-lldb-unwrapped
      install -m755 ${build}/bin/lldb-server ${build}/bin/lld \
        ${build}/bin/llvm-symbolizer ${build}/bin/llc $out/bin/
      for so in ${build}/lib/*.so*; do
        install -m644 "$so" $out/lib/
      done
      install -m644 ${build}/lib/mojo/std.mojoc $out/lib/mojo/std.mojoc

      # Jupyter kernel spec sources, consumed by mojo-jupyter-kernel.
      mkdir -p $out/share/jupyter/kernels/mojo
      install -m644 ${build}/share/jupyter-kernel/mojokernel.py \
        ${build}/share/jupyter-kernel/logo-64x64.png \
        ${build}/share/jupyter-kernel/logo.svg \
        $out/share/jupyter/kernels/mojo/

      # ld.lld is the flavor the driver asks for when it shells out;
      # wasm-ld is the flavor dev/mojo/gui's wasm pipeline links with.
      ln -s lld $out/bin/ld.lld
      ln -s lld $out/bin/wasm-ld

      # lldb links the Jammy sysroot's libedit (SONAME libedit.so.2);
      # nixpkgs ships the ABI-compatible library as libedit.so.0, same
      # bridge the old binary package used.
      ln -s ${libedit}/lib/libedit.so.0 $out/lib/libedit.so.2

      # mblack is plain Python in-tree; run it off a nixpkgs interpreter
      # rather than through bazel's hermetic CPython.
      mkdir -p $out/share/mblack
      cp -r ${build}/share/mblack-src/. $out/share/mblack/
      chmod -R u+w $out/share/mblack
      # nixpkgs' pathspec predates PathSpec.__class_getitem__ and these
      # subscripts are type annotations that CPython evaluates at import
      # time; the bare class means the same thing to a formatter.
      sed -i 's/PathSpec\[PathSpecPattern\]/PathSpec/g' \
        $out/share/mblack/mblack/files.py $out/share/mblack/mblack/__init__.py
      makeWrapper ${mblackPythonEnv}/bin/python $out/bin/mblack \
        --prefix PYTHONPATH : $out/share/mblack \
        --add-flags "-m mblack"

      # Every mojo-max.* key not named here resolves to a
      # package_root-relative default (bin/mojo, bin/lld,
      # lib/libKGENCompilerRTShared.so, ...).
      cat > $out/etc/modular/modular.cfg <<EOF
      [max]
      name = Mojo (built from source)
      version = ${version}

      [mojo-max]
      package_root = $out
      import_path = $out/lib/mojo
      lldb_path = $out/bin/mojo-lldb
      system_libs = -lrt,-ldl,-lpthread,-lm,-lz,-ltinfo
      EOF

      # The driver derives its cache and crash directories from
      # MODULAR_HOME, which lives in the read-only store; point both at
      # /tmp like the previous binary package did.
      ln -s /tmp $out/etc/modular/cache
      ln -s /tmp $out/etc/modular/crashdb

      makeWrapper $out/bin/mojo-unwrapped $out/bin/mojo \
        --set MODULAR_HOME $out/etc/modular \
        --set-default MODULAR_CRASH_REPORTING_ENABLED 0 \
        --set-default MODULAR_TELEMETRY_ENABLED 0 \
        --set-default TERMINFO_DIRS ${ncurses}/share/terminfo
      makeWrapper $out/bin/mojo-lldb-unwrapped $out/bin/mojo-lldb \
        --set MODULAR_HOME $out/etc/modular \
        --set-default TERMINFO_DIRS ${ncurses}/share/terminfo
      makeWrapper $out/bin/mojo-lsp-server-unwrapped $out/bin/mojo-lsp-server \
        --set MODULAR_HOME $out/etc/modular \
        --add-flags "-I $out/lib/mojo"

      runHook postInstall
    '';

    doInstallCheck = true;
    installCheckPhase = ''
      runHook preInstallCheck

      $out/bin/mojo --version
      $out/bin/mojo-lsp-server --version
      $out/bin/mblack --version
      $out/bin/llc --version >/dev/null
      $out/bin/wasm-ld --version >/dev/null

      cd "$(mktemp -d)"
      {
        echo 'def main():'
        echo '    print("mojo built from source")'
      } > smoke.mojo
      $out/bin/mojo run smoke.mojo | grep -q "mojo built from source"
      $out/bin/mojo build smoke.mojo -o smoke
      ./smoke | grep -q "mojo built from source"

      runHook postInstallCheck
    '';

    passthru = {inherit deps build;};

    meta = {
      description = "Mojo programming language, compiler and tools built from source";
      homepage = "https://www.modular.com/mojo";
      license = [lib.licenses.asl20 lib.licenses."llvm-exception"];
      platforms = ["x86_64-linux" "aarch64-linux"];
      maintainers = with lib.maintainers; [overby-me];
    };
  }
