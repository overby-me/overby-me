# Top-level API: build a Rust project from Cargo.lock with per-crate
# derivations. See PLAN.md and README.md.
{
  lib,
  stdenv,
  rustc,
  python3,
  fetchurl,
  writeText,
}: let
  cargoLib = import ../lib;
  buildCrate = import ./buildCrate.nix {inherit lib stdenv rustc python3 writeText;};
in
  {
    src,
    # Path to a registry index checkout (snapshot mini-index or full index).
    index,
    lockFile ? null,
    pname ? null,
    # Root package features.
    features ? [],
    noDefaultFeatures ? false,
    # Workspace member to build. Defaults to the sole member.
    roots ? null,
    # Subset of [[bin]] targets to build (names). null builds all.
    bins ? null,
    release ? true,
    # Per-crate derivation attribute merges, keyed by crate name:
    # { openssl-sys = { buildInputs = [ openssl ]; nativeBuildInputs = [ pkg-config ]; }; }
    crateOverrides ? {},
    meta ? {},
  }: let
    inherit (builtins) attrNames concatStringsSep elem filter foldl' hashString head length mapAttrs readFile substring;

    platform = cargoLib.cfg.platformFromSystem stdenv.hostPlatform.system;
    workspace = cargoLib.manifest.loadWorkspace src;
    lock = cargoLib.lock.parseLock (readFile (
      if lockFile != null
      then lockFile
      else cargoLib.manifest.joinPath src "Cargo.lock"
    ));

    rootNames =
      if roots != null
      then roots
      else if length workspace.members == 1
      then [(head workspace.members).name]
      else throw "buildCargoProject: workspace has ${toString (length workspace.members)} members; set roots = [ \"name\" ]";

    rootName =
      if length rootNames == 1
      then head rootNames
      else throw "buildCargoProject: exactly one root supported for now";

    resolved = cargoLib.resolve.resolve {
      inherit lock platform workspace;
      indexDir = index;
      roots = rootNames;
      rootFeatures = features;
      inherit noDefaultFeatures;
    };
    inherit (resolved) nodes;

    profile = {
      optLevel =
        if release
        then "3"
        else "0";
      debug = !release;
    };

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

    hashOf = id: node:
      substring 0 16 (hashString "sha256"
        "${id}:${concatStringsSep "," node.features}:${rustc.version}:v1");

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

    mkCrate = id: node: let
      isRoot = node.isWorkspaceMember && node.pkg.name == rootName;
      dedupeByName = edges:
        builtins.attrValues (foldl' (acc: e: acc // {${e.name} = e;}) {} edges);
    in
      buildCrate {
        crateName = node.pkg.name;
        inherit (node.pkg) version;
        src =
          if node.isWorkspaceMember
          then filterSrc (cargoLib.manifest.joinPath src node.meta.relDir)
          else
            fetchurl {
              name = "${node.pkg.name}-${node.pkg.version}.crate";
              url = "https://static.crates.io/crates/${node.pkg.name}/${node.pkg.name}-${node.pkg.version}.crate";
              sha256 = node.pkg.checksum;
            };
        plan =
          if node.isWorkspaceMember
          then planFor node isRoot
          else null;
        inherit (node) features;
        externs = map (e: {
          inherit (e) name;
          drv = drvs.${e.targetId};
        }) (dedupeByName (normalEdges node));
        buildExterns = map (e: {
          inherit (e) name;
          drv = drvs.${e.targetId};
        }) (dedupeByName (buildEdges node));
        depDrvs = map (i: drvs.${i}) (attrNames closureSet.${id});
        buildDepDrvs = map (i: drvs.${i}) (attrNames buildClosureSet.${id});
        linksDepDrvs = map (e: drvs.${e.targetId}) (normalEdges node);
        target = platform.triple;
        inherit profile;
        capLints = !node.isWorkspaceMember;
        buildBins = isRoot;
        crateHash = hashOf id node;
        extraAttrs =
          (crateOverrides.${node.pkg.name} or {})
          // lib.optionalAttrs isRoot {
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
          };
      };

    rootId = head resolved.rootIds;
  in
    drvs.${rootId}
