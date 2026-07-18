# Workspace manifest loading and normalization. Pure builtins.
#
# Reads local Cargo.toml files (allowed at eval time: they are source files),
# discovers workspace members, applies workspace inheritance, and normalizes
# packages into the same dependency-record shape produced by index.nix.
let
  inherit
    (builtins)
    attrNames
    concatLists
    elem
    filter
    foldl'
    fromTOML
    isAttrs
    isPath
    isString
    listToAttrs
    match
    pathExists
    readDir
    readFile
    replaceStrings
    ;

  joinPath = src: sub:
    if sub == ""
    then src
    else if isPath src
    then src + ("/" + sub)
    else "${src}/${sub}";

  loadManifest = path: fromTOML (readFile path);

  # Resolve `field = { workspace = true }` inheritance from
  # [workspace.package].
  resolveField = wsPackage: fieldName: v:
    if isAttrs v && (v.workspace or false)
    then wsPackage.${fieldName}
      or (throw "cargo manifest: ${fieldName}.workspace = true but [workspace.package] has no ${fieldName}")
    else v;

  # Normalize one dependency spec into the shared dep record shape.
  # `wsDeps` is [workspace.dependencies] for `workspace = true` merging.
  normalizeDep = wsDeps: kind: target: name: spec: let
    base = let
      b = wsDeps.${name} or (throw "cargo manifest: dependency ${name} sets workspace = true but [workspace.dependencies] has no entry for it");
    in
      if isString b
      then {version = b;}
      else b;
    s =
      if isString spec
      then {version = spec;}
      else if spec.workspace or false
      then
        base
        // {
          features = (base.features or []) ++ (spec.features or []);
          optional = spec.optional or false;
          default-features = spec."default-features" or (base."default-features" or true);
        }
      else spec;
  in {
    inherit name kind target;
    package = s.package or name;
    req = s.version or null;
    optional = s.optional or false;
    defaultFeatures = s."default-features" or (s.default_features or true);
    features = s.features or [];
    path = s.path or null;
    git = s.git or null;
    rev = s.rev or null;
    registry = s.registry or null;
  };

  # All dependency records of a manifest: plain sections plus [target.X]
  # sections, both hyphen and underscore spellings.
  sectionKinds = [
    {
      keys = ["dependencies"];
      kind = "normal";
    }
    {
      keys = ["dev-dependencies" "dev_dependencies"];
      kind = "dev";
    }
    {
      keys = ["build-dependencies" "build_dependencies"];
      kind = "build";
    }
  ];

  depsOfTables = wsDeps: target: tables:
    concatLists (map (
        sk: let
          tbl =
            foldl' (
              acc: k:
                acc // (tables.${k} or {})
            ) {}
            sk.keys;
        in
          map (name: normalizeDep wsDeps sk.kind target name tbl.${name}) (attrNames tbl)
      )
      sectionKinds);

  normalizeDeps = wsDeps: manifest: let
    plain = depsOfTables wsDeps null manifest;
    targeted = concatLists (map (
      t:
        depsOfTables wsDeps t manifest.target.${t}
    ) (attrNames (manifest.target or {})));
  in
    plain ++ targeted;

  snakeName = replaceStrings ["-"] ["_"];

  # Discover [[bin]] targets: explicit entries, src/main.rs, src/bin/*.
  discoverBins = dir: pkgName: manifest: let
    explicit =
      map (b: {
        inherit (b) name;
        path =
          b.path or (
            if pathExists (joinPath dir "src/bin/${b.name}.rs")
            then "src/bin/${b.name}.rs"
            else if pathExists (joinPath dir "src/bin/${b.name}/main.rs")
            then "src/bin/${b.name}/main.rs"
            else if b.name == pkgName && pathExists (joinPath dir "src/main.rs")
            then "src/main.rs"
            else throw "cargo manifest: cannot find source for [[bin]] ${b.name} in ${toString dir}"
          );
        requiredFeatures = b."required-features" or [];
      })
      (manifest.bin or []);
    explicitNames = map (b: b.name) explicit;
    explicitPaths = map (b: b.path) explicit;
    autoMain =
      if pathExists (joinPath dir "src/main.rs") && !(elem "src/main.rs" explicitPaths) && !(elem pkgName explicitNames)
      then [
        {
          name = pkgName;
          path = "src/main.rs";
          requiredFeatures = [];
        }
      ]
      else [];
    binDir = joinPath dir "src/bin";
    autoBinDir =
      if !pathExists binDir
      then []
      else let
        entries = readDir binDir;
      in
        concatLists (map (
          n: let
            t = entries.${n};
            mrs = match "(.*)\\.rs" n;
          in
            if t == "regular" && mrs != null && !(elem (builtins.head mrs) explicitNames)
            then [
              {
                name = builtins.head mrs;
                path = "src/bin/${n}";
                requiredFeatures = [];
              }
            ]
            else if t == "directory" && pathExists (joinPath binDir "${n}/main.rs") && !(elem n explicitNames)
            then [
              {
                name = n;
                path = "src/bin/${n}/main.rs";
                requiredFeatures = [];
              }
            ]
            else []
        ) (attrNames entries));
  in
    explicit ++ autoMain ++ autoBinDir;

  discoverLib = dir: pkgName: manifest: let
    explicit = manifest.lib or null;
    hasAuto = pathExists (joinPath dir "src/lib.rs");
  in
    if explicit == null && !hasAuto
    then null
    else let
      e =
        if explicit == null
        then {}
        else explicit;
      procMacro = e."proc-macro" or (e.proc_macro or false);
    in {
      name = e.name or (snakeName pkgName);
      path = e.path or "src/lib.rs";
      inherit procMacro;
      crateTypes =
        e."crate-type"
        or (e.crate_type or (
          if procMacro
          then ["proc-macro"]
          else ["lib"]
        ));
    };

  # Normalize a member package.
  # dir: absolute path (path or string) to the package directory.
  # relDir: directory relative to the workspace root ("" for the root).
  normalizePackage = {
    dir,
    relDir ? "",
    manifest,
    workspaceManifest ? {},
  }: let
    wsPackage = (workspaceManifest.workspace or {}).package or {};
    wsDeps = (workspaceManifest.workspace or {}).dependencies or {};
    pkg = manifest.package;
    field = name: default: resolveField wsPackage name (pkg.${name} or default);
    inherit (pkg) name;
  in {
    inherit name relDir;
    version = field "version" "0.0.0";
    edition = field "edition" "2015";
    description = field "description" "";
    license = field "license" "";
    repository = field "repository" "";
    links = pkg.links or null;
    hasBuildScript = pkg ? build || pathExists (joinPath dir "build.rs");
    buildScript =
      if pkg ? build && isString pkg.build
      then pkg.build
      else if pathExists (joinPath dir "build.rs")
      then "build.rs"
      else null;
    deps = normalizeDeps wsDeps manifest;
    features = manifest.features or {};
    lib = discoverLib dir name manifest;
    bins = discoverBins dir name manifest;
    resolver = manifest.workspace.resolver or (pkg.resolver or null);
  };

  # Expand [workspace] member globs. Supports literals and a single
  # trailing "/*" component (the common cases).
  expandMembers = src: patterns: excludes: let
    expand = pat: let
      mGlob = match "(.*)/\\*" pat;
    in
      if pat == "*"
      then dirsWithManifest src ""
      else if mGlob != null
      then dirsWithManifest src (builtins.head mGlob)
      else if pathExists (joinPath src "${pat}/Cargo.toml")
      then [pat]
      else throw "cargo manifest: workspace member ${pat} has no Cargo.toml";
    dirsWithManifest = root: prefix: let
      base = joinPath root prefix;
      entries = readDir base;
    in
      filter (d: d != null) (map (
        n:
          if entries.${n} == "directory" && pathExists (joinPath base "${n}/Cargo.toml")
          then
            (
              if prefix == ""
              then n
              else "${prefix}/${n}"
            )
          else null
      ) (attrNames entries));
    all = concatLists (map expand patterns);
  in
    filter (d: !(elem d excludes)) all;

  # Load a workspace (or standalone package) from a source root.
  loadWorkspace = src: let
    rootManifest = loadManifest (joinPath src "Cargo.toml");
    hasWs = rootManifest ? workspace;
    memberDirs =
      if hasWs
      then
        expandMembers src (rootManifest.workspace.members or [])
        (rootManifest.workspace.exclude or [])
      else [];
    rootIsPkg = rootManifest ? package;
    dirs =
      (
        if rootIsPkg
        then [""]
        else []
      )
      ++ filter (d: d != "" && d != ".") memberDirs;
    members =
      map (
        d: let
          manifest =
            if d == ""
            then rootManifest
            else loadManifest (joinPath src "${d}/Cargo.toml");
        in
          normalizePackage {
            dir = joinPath src d;
            relDir = d;
            inherit manifest;
            workspaceManifest = rootManifest;
          }
      )
      dirs;
  in {
    inherit rootManifest members;
    byName = listToAttrs (map (m: {
        inherit (m) name;
        value = m;
      })
      members);
  };
in {
  inherit loadManifest loadWorkspace normalizePackage normalizeDeps snakeName joinPath;
}
