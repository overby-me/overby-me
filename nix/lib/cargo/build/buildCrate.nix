# One crate, one derivation. rustc invoked directly by crate_builder.py;
# no cargo in the sandbox.
{
  lib,
  stdenv,
  rustc,
  nushell,
  writeText,
}: {
  crateName,
  version,
  src,
  # Eval-provided build plan (workspace members). null means the plan is
  # computed inside the sandbox from the published manifest (registry crates).
  plan ? null,
  features ? [],
  # Direct dependencies: [{ name (extern name, pre-rename), drv }]
  externs ? [],
  buildExterns ? [],
  # Transitive normal-dep drvs (rlib search path + native link collection).
  depDrvs ? [],
  buildDepDrvs ? [],
  # Direct normal deps whose links metadata feeds DEP_* env vars.
  linksDepDrvs ? [],
  target,
  # Cross-compilation: pass --target for lib/bin compiles (build scripts
  # and proc-macros stay host); hostTriple feeds the HOST env var.
  crossTarget ? null,
  hostTriple ? null,
  # Path to a file with the toolchain's `rustc --print cfg` output.
  rustcCfgFile ? null,
  profile,
  capLints ? true,
  buildBins ? false,
  bins ? null,
  crateHash,
  # Extra rustc flags for link steps (lib/dylib/bin compiles), e.g. an
  # alternative linker via ["-C" "link-arg=-B/path/with/ld"].
  linkArgs ? [],
  # Extra flags for every rustc invocation (including build scripts), e.g.
  # ["-Zcodegen-backend=cranelift"] with a nightly toolchain.
  rustcFlags ? [],
  # Toolchain override (a package providing bin/rustc); defaults to the
  # rustc this library was imported with.
  toolchain ? null,
  # P7 pipelining: emit only crate metadata (.rmeta) when the crate is
  # eligible (lib target, no build script, not a proc-macro); ineligible
  # crates fall back to a full build inside this derivation.
  emitMetadataOnly ? false,
  # In full mode: the crate's rmeta derivation. When that derivation fell
  # back to a full build, artifacts are copied instead of recompiling.
  fallbackFrom ? null,
  # Bin links need real rlibs even when lib compiles use rmeta.
  linkExterns ? null,
  linkDepOuts ? null,
  # crateOverrides merge (buildInputs, env, patches, ...).
  extraAttrs ? {},
}: let
  config = writeText "cargo-nix-config-${crateName}-${version}.json" (builtins.toJSON ({
      inherit plan features target profile capLints buildBins bins crateHash;
      rustcCfgFile =
        if rustcCfgFile == null
        then null
        else "${rustcCfgFile}";
      externs =
        map (e: {
          inherit (e) name;
          renamed = e.renamed or false;
          out = "${e.drv}";
        })
        externs;
      buildExterns =
        map (e: {
          inherit (e) name;
          renamed = e.renamed or false;
          out = "${e.drv}";
        })
        buildExterns;
      depOuts = map (d: "${d}") depDrvs;
      buildDepOuts = map (d: "${d}") buildDepDrvs;
      linksDeps = map (d: "${d}") linksDepDrvs;
    }
    // lib.optionalAttrs (linkArgs != []) {inherit linkArgs;}
    // lib.optionalAttrs (rustcFlags != []) {inherit rustcFlags;}
    // lib.optionalAttrs emitMetadataOnly {inherit emitMetadataOnly;}
    // lib.optionalAttrs (crossTarget != null) {inherit crossTarget;}
    // lib.optionalAttrs (hostTriple != null) {inherit hostTriple;}
    // lib.optionalAttrs (fallbackFrom != null) {fallbackFrom = "${fallbackFrom}";}
    // lib.optionalAttrs (linkExterns != null) {
      linkExterns =
        map (e: {
          inherit (e) name;
          renamed = e.renamed or false;
          out = "${e.drv}";
        })
        linkExterns;
    }
    // lib.optionalAttrs (linkDepOuts != null) {
      linkDepOuts = map (d: "${d}") linkDepOuts;
    }));

  base = {
    pname = crateName;
    inherit version src;

    nativeBuildInputs = [
      (
        if toolchain != null
        then toolchain
        else rustc
      )
      nushell
    ];

    # Registry .crate files are gzipped tarballs with an unknown extension;
    # workspace member sources are plain directories.
    unpackCmd = ''
      if [ -f "$curSrc" ]; then
        tar xzf "$curSrc"
      else
        cp -pr --reflink=auto -- "$curSrc" "$(stripHash "$curSrc")"
      fi
    '';

    dontConfigure = true;
    # rlibs are ar archives of bitcode/objects; stripping mangles them.
    dontStrip = true;

    buildPhase = ''
      runHook preBuild
      nu --no-config-file ${./crate-builder.nu} ${config}
      runHook postBuild
    '';

    # The builder writes $out itself; the install phase only exists so that
    # preInstall/postInstall overrides (e.g. extra symlinks) keep working.
    installPhase = ''
      runHook preInstall
      runHook postInstall
    '';
  };
in
  stdenv.mkDerivation (base
    // extraAttrs
    // lib.optionalAttrs (extraAttrs ? nativeBuildInputs) {
      nativeBuildInputs = base.nativeBuildInputs ++ extraAttrs.nativeBuildInputs;
    })
