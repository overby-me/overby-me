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
  profile,
  capLints ? true,
  buildBins ? false,
  bins ? null,
  crateHash,
  # crateOverrides merge (buildInputs, env, patches, ...).
  extraAttrs ? {},
}: let
  config = writeText "cargo-nix-config-${crateName}-${version}.json" (builtins.toJSON {
    inherit plan features target profile capLints buildBins bins crateHash;
    externs =
      map (e: {
        inherit (e) name;
        out = "${e.drv}";
      })
      externs;
    buildExterns =
      map (e: {
        inherit (e) name;
        out = "${e.drv}";
      })
      buildExterns;
    depOuts = map (d: "${d}") depDrvs;
    buildDepOuts = map (d: "${d}") buildDepDrvs;
    linksDeps = map (d: "${d}") linksDepDrvs;
  });

  base = {
    pname = crateName;
    inherit version src;

    nativeBuildInputs = [rustc nushell];

    # Registry .crate files are gzipped tarballs with an unknown extension.
    unpackCmd = ''tar xzf "$curSrc"'';

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
