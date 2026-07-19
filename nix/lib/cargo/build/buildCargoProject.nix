# Top-level API: build a Rust project from Cargo.lock with per-crate
# derivations. See PLAN.md and README.md.
{
  lib,
  stdenv,
  rustc,
  nushell,
  fetchurl,
  writeText,
  symlinkJoin,
  runCommand,
}: let
  cargoLib = import ../lib;
  buildCrate = import ./buildCrate.nix {inherit lib stdenv rustc nushell writeText;};
in
  {
    src,
    # Path to a registry index checkout (snapshot mini-index or full index).
    index,
    # Set when the workspace manifest is not at the root of src (projects
    # with path dependencies on sibling directories).
    manifestDir ? "",
    lockFile ? null,
    pname ? null,
    # Version for the aggregate output of multi-root workspace builds
    # (single-root builds take the version from the crate manifest).
    version ? null,
    # Root package features.
    features ? [],
    noDefaultFeatures ? false,
    # Workspace members to build. Defaults to all members.
    roots ? null,
    # Subset of [[bin]] targets to build (names). null builds all.
    bins ? null,
    release ? true,
    # P7 (experimental): rmeta pipelining. Dependents compile against
    # dependency crate metadata so codegen leaves the critical path; bins
    # still link real rlibs. Requires a nightly toolchain with
    # -Zalways-encode-mir in rustcFlags: stable's standalone
    # --emit=metadata produces check-grade rmeta without the optimized
    # MIR dependents need for codegen.
    pipeline ? false,
    # Per-crate derivation attribute merges, keyed by crate name:
    # { openssl-sys = { buildInputs = [ openssl ]; nativeBuildInputs = [ pkg-config ]; }; }
    crateOverrides ? {},
    # Alternative linker package (e.g. pkgs.wild or pkgs.mold); its main
    # program is exposed as `ld` to cc via -B.
    linker ? null,
    # Extra flags for every rustc invocation, e.g.
    # ["-Zcodegen-backend=cranelift"] together with a nightly toolchain.
    rustcFlags ? [],
    # Toolchain override (e.g. rust-bin.nightly.latest.default with the
    # rustc-codegen-cranelift-preview extension).
    toolchain ? null,
    # Extra derivation attrs for the root crate (postInstall, env, ...).
    rootAttrs ? {},
    meta ? {},
  }: let
    inherit (builtins) attrNames concatStringsSep elem filter foldl' hashString head length mapAttrs readFile substring;

    platform = cargoLib.cfg.platformFromSystem stdenv.hostPlatform.system;
    workspace = cargoLib.manifest.loadWorkspace {inherit src manifestDir;};
    lock = cargoLib.lock.parseLock (readFile (
      if lockFile != null
      then lockFile
      else cargoLib.manifest.joinPath (cargoLib.manifest.joinPath src manifestDir) "Cargo.lock"
    ));

    rootNames =
      if roots != null
      then roots
      else map (m: m.name) workspace.members;

    singleRoot = length rootNames == 1;

    fetchCrate = pkg:
      fetchurl {
        name = "${pkg.name}-${pkg.version}.crate";
        url = "https://static.crates.io/crates/${pkg.name}/${pkg.name}-${pkg.version}.crate";
        sha256 = pkg.checksum;
      };

    # P5: the compiler cfg set is a pure function of the toolchain; run
    # `rustc --print cfg` once instead of in every build-script sandbox.
    rustcCfgFile =
      runCommand "rustc-cfg-${effectiveRustcVersion}" {
        nativeBuildInputs = [
          (
            if toolchain != null
            then toolchain
            else rustc
          )
        ];
      } ''
        rustc --print cfg > $out
      '';

    resolved = cargoLib.resolve.resolve {
      inherit lock platform workspace;
      indexDir = index;
      roots = rootNames;
      rootFeatures = features;
      inherit noDefaultFeatures;
    };
    inherit (resolved) nodes;

    profiles = cargoLib.profile.mkProfiles {
      inherit (workspace) rootManifest;
      inherit release;
    };

    linkerDir =
      if linker == null
      then null
      else
        runCommand "cargo-nix-ld" {} ''
          mkdir -p $out/bin
          ln -s ${lib.getExe linker} $out/bin/ld
        '';
    linkArgs =
      if linker == null
      then []
      else ["-C" "link-arg=-B${linkerDir}/bin"];
    normalEdges = node: filter (e: e.kind == "normal") node.edges;
    buildEdges = node: filter (e: e.kind == "build") node.edges;

    # Transitive closure of normal deps per node (attrset of ids).
    closureSet =
      mapAttrs (
        _id: node:
          foldl' (
            acc: e:
              acc // {${e.targetId} = true;} // closureSet.${e.targetId}
          ) {} (normalEdges node)
      )
      nodes;

    # For the build script compile: build deps plus their normal closures.
    buildClosureSet =
      mapAttrs (
        _id: node:
          foldl' (
            acc: e:
              acc // {${e.targetId} = true;} // closureSet.${e.targetId}
          ) {} (buildEdges node)
      )
      nodes;

    effectiveRustcVersion =
      if toolchain != null
      then toolchain.version
      else rustc.version;

    hashOf = id: node:
      substring 0 16 (hashString "sha256"
        "${id}:${concatStringsSep "," node.features}:${effectiveRustcVersion}:v1");

    filterSrc = dir:
      builtins.filterSource (
        path: _type: let
          bn = baseNameOf path;
        in
          bn != "target" && bn != ".git" && bn != ".jj" && bn != "result"
      )
      dir;

    planFor = node: isRoot: {
      name = node.pkg.name;
      inherit (node.pkg) version;
      inherit (node.meta) edition links description license repository;
      authors = [];
      rustVersion = "";
      inherit (node.meta) lib;
      build = node.meta.buildScript;
      bins =
        if !isRoot
        then []
        else
          map (b: {inherit (b) name path;}) (filter (
              b:
                (bins == null || elem b.name bins)
                && (b.requiredFeatures == [] || builtins.all (f: elem f node.features) b.requiredFeatures)
            )
            node.meta.bins);
    };

    drvs = mapAttrs mkCrate nodes;

    # Metadata-only derivations for pipelining. Members are excluded
    # (they are roots and build bins); everything else gets one, and the
    # sandbox decides eligibility (falling back to a full build for
    # proc-macros and build-script crates).
    rmetaDrvs =
      mapAttrs (
        id: node:
          if !pipeline || node.isWorkspaceMember
          then null
          else mkRmeta id node
      )
      nodes;

    # The derivation a lib compile should take its extern for `id` from.
    metaDrvOf = id:
      if rmetaDrvs.${id} != null
      then rmetaDrvs.${id}
      else drvs.${id};

    mkRmeta = id: node: let
      dedupeByName = edges:
        builtins.attrValues (foldl' (acc: e: acc // {${e.name} = e;}) {} edges);
    in
      buildCrate {
        crateName = "${node.pkg.name}-rmeta";
        inherit (node.pkg) version;
        src =
          if node.pkg.sourceInfo.type != "registry"
          then filterSrc (cargoLib.manifest.joinPath (node.meta.srcBase or src) node.meta.relDir)
          else fetchCrate node.pkg;
        plan =
          if node.pkg.sourceInfo.type != "registry"
          then planFor node false
          else null;
        inherit (node) features;
        emitMetadataOnly = true;
        externs = map (e: {
          inherit (e) name;
          renamed = e.name != e.package;
          drv = metaDrvOf e.targetId;
        }) (dedupeByName (normalEdges node));
        buildExterns = map (e: {
          inherit (e) name;
          renamed = e.name != e.package;
          drv = drvs.${e.targetId};
        }) (dedupeByName (buildEdges node));
        depDrvs = map metaDrvOf (attrNames closureSet.${id});
        buildDepDrvs = map (i: drvs.${i}) (attrNames buildClosureSet.${id});
        linksDepDrvs = map (e: metaDrvOf e.targetId) (normalEdges node);
        target = platform.triple;
        inherit rustcCfgFile;
        profile = profiles.forPackage node.pkg.name node.isWorkspaceMember;
        inherit linkArgs rustcFlags toolchain;
        capLints = true;
        buildBins = false;
        crateHash = hashOf id node;
        extraAttrs = crateOverrides.${node.pkg.name} or {};
      };

    mkCrate = id: node: let
      isRoot = node.isWorkspaceMember && elem node.pkg.name rootNames;
      dedupeByName = edges:
        builtins.attrValues (foldl' (acc: e: acc // {${e.name} = e;}) {} edges);
    in
      buildCrate {
        crateName = node.pkg.name;
        inherit (node.pkg) version;
        src =
          if node.pkg.sourceInfo.type != "registry"
          then filterSrc (cargoLib.manifest.joinPath (node.meta.srcBase or src) node.meta.relDir)
          else fetchCrate node.pkg;
        plan =
          if node.pkg.sourceInfo.type != "registry"
          then planFor node isRoot
          else null;
        inherit (node) features;
        externs = map (e: {
          inherit (e) name;
          renamed = e.name != e.package;
          drv =
            if pipeline
            then metaDrvOf e.targetId
            else drvs.${e.targetId};
        }) (dedupeByName (normalEdges node));
        linkExterns =
          if !pipeline
          then null
          else
            map (e: {
              inherit (e) name;
              renamed = e.name != e.package;
              drv = drvs.${e.targetId};
            }) (dedupeByName (normalEdges node));
        linkDepOuts =
          if !pipeline
          then null
          else map (i: drvs.${i}) (attrNames closureSet.${id});
        fallbackFrom =
          if pipeline && !node.isWorkspaceMember
          then rmetaDrvs.${id}
          else null;
        emitMetadataOnly = false;
        buildExterns = map (e: {
          inherit (e) name;
          renamed = e.name != e.package;
          drv = drvs.${e.targetId};
        }) (dedupeByName (buildEdges node));
        depDrvs = map (
          i:
            if pipeline
            then metaDrvOf i
            else drvs.${i}
        ) (attrNames closureSet.${id});
        buildDepDrvs = map (i: drvs.${i}) (attrNames buildClosureSet.${id});
        linksDepDrvs = map (
          e:
            if pipeline
            then metaDrvOf e.targetId
            else drvs.${e.targetId}
        ) (normalEdges node);
        target = platform.triple;
        inherit rustcCfgFile;
        profile = profiles.forPackage node.pkg.name node.isWorkspaceMember;
        inherit linkArgs rustcFlags toolchain;
        capLints = !node.isWorkspaceMember;
        buildBins = isRoot;
        crateHash = hashOf id node;
        extraAttrs =
          (crateOverrides.${node.pkg.name} or {})
          // lib.optionalAttrs (isRoot && singleRoot) ({
              pname =
                if pname != null
                then pname
                else node.pkg.name;
              inherit meta;
              passthru =
                {
                  crates = drvs;
                  inherit nodes;
                }
                // (crateOverrides.${node.pkg.name}.passthru or {});
            }
            // rootAttrs);
      };

    rootId = head resolved.rootIds;
  in
    if singleRoot
    then drvs.${rootId}
    else
      symlinkJoin ({
          name =
            if pname != null
            then pname
            else throw "buildCargoProject: set pname when building several workspace members";
          version =
            if version != null
            then version
            else "0.0.0";
          paths = map (i: drvs.${i}) resolved.rootIds;
          passthru = {
            crates = drvs;
            inherit nodes;
          };
          inherit meta;
        }
        // rootAttrs)
