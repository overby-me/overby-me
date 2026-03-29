{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  fixDarwinDylibNames,
  unzip,
  zstd,
  libedit,
  zlib,
  curl,
  libbsd,
  python3,
  darwin ? {},
}: let
  mblackPythonEnv = python3.withPackages (ps:
    with ps; [
      click
      mypy-extensions
      packaging
      pathspec
      platformdirs
      tomli
      typing-extensions
    ]);

  inherit (stdenv) isDarwin isLinux;
  condaPlatform =
    if isDarwin
    then "osx-arm64"
    else "linux-64";
in
  stdenv.mkDerivation rec {
    pname = "mojo";
    version = "26.2.0";

    srcs =
      [
        (fetchurl {
          url = "https://conda.modular.com/max/${condaPlatform}/mojo-compiler-0.${version}-release.conda";
          sha256 =
            if isDarwin
            then "sha256-6Cy3zhp1aHaiM9Nk1Tl63KWtfiD/9SBMHH6Trv5FSbU="
            else "sha256-5T8w1ypl5rCmd0XmmI+XjY9uzBRTFCBjHJcZ65x7nCs=";
        })
        (fetchurl {
          url = "https://conda.modular.com/max/${condaPlatform}/mojo-0.${version}-release.conda";
          sha256 =
            if isDarwin
            then "sha256-41uNNFZLMBicWphZkEZrQoP54diXuvI/6d28vJKDRZw="
            else "sha256-TXBHRmwT2SscjrGcbSA6hQOvvSsa1j61KJqPS79NKUU=";
        })
        (fetchurl {
          url = "https://repo.prefix.dev/max/noarch/mblack-${version}-release.conda";
          sha256 = "sha256-PC/OuJzvyomdzXyPJqYgKsrLKgc+TLLvNlUUpGXQHc0=";
        })
      ]
      # Using nixpkgs ncurses, mojo fails with error:
      # version `NCURSES6_5.0.19991023' not found (required by <NIX-STORE-PATH>/lib/liblldb20.0.0git.so)
      # So let's use the ncurses from Conda (Linux only — macOS uses system ncurses)
      ++ lib.optionals isLinux [
        (fetchurl {
          url = "https://conda.anaconda.org/conda-forge/linux-64/ncurses-6.5-h2d0b736_3.conda";
          sha256 = "sha256-P94pMjL6P8qYY14RZ95rfH/ag8ryS51skeye77T01YY=";
        })
      ];

    sourceRoot = ".";
    preferLocalBuild = true;

    nativeBuildInputs =
      [
        unzip
        zstd
      ]
      ++ lib.optionals isLinux [
        autoPatchelfHook
      ]
      ++ lib.optionals isDarwin [
        fixDarwinDylibNames
      ];

    buildInputs =
      lib.optionals isLinux [
        stdenv.cc.cc.lib
        libedit
        zlib
        curl
        libbsd
      ]
      ++ lib.optionals isDarwin [
        darwin.apple_sdk.frameworks.CoreFoundation
        darwin.apple_sdk.frameworks.CoreServices
        darwin.apple_sdk.frameworks.Security
      ];

    unpackPhase = ''
      for src in $srcs; do
        unzip -o $src
        tar --zstd -xf pkg-*.tar.zst
        rm pkg-*.tar.zst
      done
    '';

    installPhase = let
      # ── modular.cfg: Linux ─────────────────────────────────────
      linuxModularCfg = ''
        [max]
        cache_dir = $out/share/max/.max_cache
        driver_lib = $out/lib/libDeviceDriver.so
        enable_compile_progress = true
        enable_model_ir_cache = true
        engine_lib = $out/lib/libmodular-framework-common.so
        graph_lib = $out/lib/libmof.so
        name = MAX Platform
        path = $out
        serve_lib = $out/lib/libServeRTCAPI.so
        torch_ext_lib = $out/lib/libmodular-framework-torch-ext.so
        version = ${version}

        [mojo-max]
        compilerrt_path = $out/lib/libKGENCompilerRTShared.so
        mgprt_path = $out/lib/libMGPRT.so
        atenrt_path = $out/lib/libATenRT.so
        shared_libs = $out/lib/libAsyncRTMojoBindings.so,$out/lib/libAsyncRTRuntimeGlobals.so,$out/lib/libMSupportGlobals.so,-Xlinker,-rpath,-Xlinker,$out/lib
        driver_path = $out/bin/mojo
        import_path = $out/lib/mojo
        jupyter_path = $out/lib/libMojoJupyter.so
        lldb_path = $out/bin/mojo-lldb
        lldb_plugin_path = $out/lib/libMojoLLDB.so
        lldb_visualizers_path = $out/lib/lldb-visualizers
        lldb_vscode_path = $out/bin/mojo-lldb-dap
        lsp_server_path = $out/bin/mojo-lsp-server
        mblack_path = $out/bin/mblack
        orcrt_path = $out/lib/liborc_rt.a
        repl_entry_point = $out/lib/mojo-repl-entry-point
        system_libs = -lrt,-ldl,-lpthread,-lm,-lz,-ltinfo
        test_executor_path = $out/lib/mojo-test-executor
      '';

      # ── modular.cfg: macOS ─────────────────────────────────────
      darwinModularCfg = ''
        [max]
        cache_dir = $out/share/max/.max_cache
        enable_model_ir_cache = true
        name = MAX Platform
        path = $out
        version = ${version}

        [mojo-max]
        compilerrt_path = $out/lib/libKGENCompilerRTShared.dylib
        mgprt_path = $out/lib/libMGPRT.dylib
        shared_libs = $out/lib/libAsyncRTMojoBindings.dylib,-Xlinker,-rpath,-Xlinker,$out/lib
        driver_path = $out/bin/mojo
        import_path = $out/lib/mojo
        jupyter_path = $out/lib/libMojoJupyter.dylib
        lldb_path = $out/bin/mojo-lldb
        lldb_plugin_path = $out/lib/libMojoLLDB.dylib
        lldb_visualizers_path = $out/lib/lldb-visualizers
        lldb_vscode_path = $out/bin/mojo-lldb-dap
        lsp_server_path = $out/bin/mojo-lsp-server
        mblack_path = $out/bin/mblack
        repl_entry_point = $out/lib/mojo-repl-entry-point
        lld_path = $out/bin/lld
      '';

      modularCfg =
        if isDarwin
        then darwinModularCfg
        else linuxModularCfg;
    in ''
      mkdir -p $out
      cp -r lib/ $out/lib/
      cp -r bin/ $out/bin/
      cp -r share/ $out/share

      # Install mblack (Mojo formatter)
      siteDir=$out/lib/${mblackPythonEnv.python.libPrefix}/site-packages
      mkdir -p $siteDir
      cp -r site-packages/* $siteDir/
      cat > $out/bin/mblack << EOF
      #!${stdenv.shell}
      export PYTHONPATH=$siteDir:\$PYTHONPATH
      exec ${mblackPythonEnv}/bin/python -m mblack "\$@"
      EOF
      chmod +x $out/bin/mblack

      ${lib.optionalString isLinux ''
        ln -s ${libedit}/lib/libedit.so.0 $out/lib/libedit.so.2
      ''}

      ${lib.optionalString isDarwin ''
        # Fix dylib install names and rpaths for macOS
        for dylib in $out/lib/*.dylib; do
          install_name_tool -id "$dylib" "$dylib" 2>/dev/null || true
        done

        # Rewrite rpaths in binaries and dylibs to point to $out/lib
        for f in $out/bin/* $out/lib/*.dylib; do
          [ -f "$f" ] || continue
          # Remove any existing conda-prefix rpaths
          for rpath in $(otool -l "$f" 2>/dev/null | grep -A2 LC_RPATH | grep 'path ' | awk '{print $2}'); do
            install_name_tool -delete_rpath "$rpath" "$f" 2>/dev/null || true
          done
          # Add our lib directory
          install_name_tool -add_rpath "$out/lib" "$f" 2>/dev/null || true
        done

        # Rewrite references to conda-prefix dylibs in all Mach-O files
        for f in $out/bin/* $out/lib/*.dylib; do
          [ -f "$f" ] || continue
          for dep in $(otool -L "$f" 2>/dev/null | tail -n +2 | awk '{print $1}' | grep -v '^/usr/lib\|^/System\|^@'); do
            base=$(basename "$dep")
            if [ -f "$out/lib/$base" ]; then
              install_name_tool -change "$dep" "$out/lib/$base" "$f" 2>/dev/null || true
            fi
          done
        done
      ''}

      # /etc/modular/modular.cfg contains hardcoded paths to libs
      mkdir -p $out/etc/modular
      cat > $out/etc/modular/modular.cfg << EOF
      ${modularCfg}
      EOF

      # Create mojo wrapper that uses generated modular.cfg
      mkdir -p $out/bin
      mv $out/bin/mojo $out/bin/mojo-unwrapped
      cat > $out/bin/mojo << EOF
      #!${stdenv.shell}
      export MODULAR_HOME=$out/etc/modular
      ${lib.optionalString isLinux "export TERMINFO_DIRS=$out/share/terminfo"}
      exec $out/bin/mojo-unwrapped "\$@"
      EOF
      chmod +x $out/bin/mojo

      # Create mojo-lldb wrapper that uses generated modular.cfg
      mkdir -p $out/bin
      mv $out/bin/mojo-lldb $out/bin/mojo-lldb-unwrapped
      cat > $out/bin/mojo-lldb << EOF
      #!${stdenv.shell}
      export MODULAR_HOME=$out/etc/modular
      ${lib.optionalString isLinux "export TERMINFO_DIRS=$out/share/terminfo"}
      exec $out/bin/mojo-lldb-unwrapped "\$@"
      EOF
      chmod +x $out/bin/mojo-lldb

      # Create mojo-lsp-server wrapper that uses generated modular.cfg
      mv $out/bin/mojo-lsp-server $out/bin/mojo-lsp-server-unwrapped
      cat > $out/bin/mojo-lsp-server << EOF
      #!${stdenv.shell}
      export MODULAR_HOME=$out/etc/modular
      exec $out/bin/mojo-lsp-server-unwrapped -I $out/lib/mojo "\$@"
      EOF
      chmod +x $out/bin/mojo-lsp-server

      # /etc/modular/crashdb needs to be mutable
      ln -s /tmp/ $out/etc/modular/crashdb

      # /etc/modular/cache needs to be mutable
      ln -s /tmp/ $out/etc/modular/cache
    '';

    doInstallCheck = true;
    installCheckPhase = ''
      $out/bin/mojo --version
      $out/bin/mojo-lldb --version
      $out/bin/mojo-lsp-server --version
    '';

    meta = with lib; {
      description = "Mojo Programming Language";
      homepage = "https://www.modular.com/mojo";
      platforms = ["x86_64-linux" "aarch64-darwin"];
      maintainers = with lib.maintainers; [noverby];
      license = licenses.unfree;
    };
  }
