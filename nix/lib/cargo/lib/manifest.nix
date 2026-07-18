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
    split
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
    fromWs = !(isString spec) && (spec.workspace or false);
    s =
      if isString spec
      then {version = spec;}
      else if fromWs
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
    # Paths from [workspace.dependencies] are relative to the workspace
    # root, not the member directory.
    pathBase =
      if fromWs && (s.path or null) != null
      then "workspace"
      else "package";
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

  # Cargo.toml is polymorphic in places (and real-world manifests abuse
  # it further); coerce defensively instead of crashing mid-eval.
  asString = v:
    if isString v
    then v
    else toString v;
  asList = v:
    if builtins.isList v
    then v
    else [v];

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
      (asList (manifest.bin or []));
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
      # crate-type is specified as a list but appears as a bare string in
      # the wild; accept both.
      crateTypes = asList (
        e."crate-type"
        or (e.crate_type or (
          if procMacro
          then ["proc-macro"]
          else ["lib"]
        ))
      );
    };

  # Normalize a member package.
  # dir: absolute path (path or string) to the package directory.
  # relDir: directory relative to the workspace root ("" for the root).
  normalizePackage = {
    dir,
    relDir ? "",
    # Directory (relative to src) of the workspace root this package
    # inherits from; base for workspace-relative dependency paths.
    wsRelDir ? "",
    manifest,
    workspaceManifest ? {},
  }: let
    wsPackage = (workspaceManifest.workspace or {}).package or {};
    wsDeps = (workspaceManifest.workspace or {}).dependencies or {};
    pkg = manifest.package;
    field = name: default: resolveField wsPackage name (pkg.${name} or default);
    inherit (pkg) name;
  in
    # Per-field error context: lazy fields forced later still identify
    # their crate when a shape surprise throws.
    builtins.mapAttrs (_: builtins.addErrorContext "while normalizing cargo package ${name} (${toString dir})") {
      inherit name relDir wsRelDir;
      version = asString (field "version" "0.0.0");
      edition = asString (field "edition" "2015");
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

  # Glob matching for workspace member/exclude patterns:
  # `*` within a path segment, `**` across segments, `?` single char.
  globMatch = pat: s: let
    esc =
      replaceStrings
      ["\\" "." "[" "]" "(" ")" "{" "}" "+" "^" "$" "|"]
      ["\\\\" "\\." "\\[" "\\]" "\\(" "\\)" "\\{" "\\}" "\\+" "\\^" "\\$" "\\|"]
      pat;
    gs = replaceStrings ["**"] ["@@GLOBSTAR@@"] esc;
    star = replaceStrings ["*"] ["[^/]*"] gs;
    qm = replaceStrings ["?"] ["[^/]"] star;
    rx = replaceStrings ["@@GLOBSTAR@@"] [".*"] qm;
  in
    match rx s != null;

  hasGlob = pat: match ".*[*?].*" pat != null;

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
    excluded = d:
      builtins.any (
        e:
          if hasGlob e
          then globMatch e d
          else e == d
      )
      excludes;
  in
    filter (d: !(excluded d)) all;

  # Lexically normalize a relative path: resolve "." and "..". Throws when
  # the path escapes the source root, since such a path cannot exist inside
  # a filtered flake source; callers must widen `src` and set `manifestDir`.
  normalizeRel = p: let
    parts = filter (s: isString s && s != "" && s != ".") (split "/" p);
    step = acc: part:
      if part == ".."
      then
        (
          if acc == []
          then throw "cargo manifest: path ${p} escapes the source root; pass a wider src with manifestDir"
          else builtins.genList (builtins.elemAt acc) (builtins.length acc - 1)
        )
      else acc ++ [part];
  in
    builtins.concatStringsSep "/" (foldl' step [] parts);

  # Load a workspace (or standalone package). Accepts either a source root
  # or { src, manifestDir } when the workspace manifest is not at the root
  # of src (needed for path dependencies on sibling projects).
  #
  # Result: members (workspace members), pathPackages (packages reached via
  # path dependencies outside the workspace), and byName over both. All
  # relDir values are relative to src.
  loadWorkspace = arg: let
    src =
      if isAttrs arg
      then arg.src
      else arg;
    manifestDir =
      if isAttrs arg
      then arg.manifestDir or ""
      else "";
    base = joinPath src manifestDir;
    rootManifest = loadManifest (joinPath base "Cargo.toml");
    hasWs = rootManifest ? workspace;
    memberDirs =
      if hasWs
      then
        expandMembers base (rootManifest.workspace.members or [])
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
            else loadManifest (joinPath base "${d}/Cargo.toml");
          relDir = normalizeRel (
            if manifestDir == ""
            then d
            else "${manifestDir}/${d}"
          );
        in
          normalizePackage {
            dir = joinPath src relDir;
            wsRelDir = normalizeRel manifestDir;
            inherit relDir manifest;
            workspaceManifest = rootManifest;
          }
      )
      dirs;

    # Path dependencies reaching outside the workspace: walk them
    # transitively, loading each target as a standalone package. Inheritance
    # is resolved against the target's own manifest (a standalone package or
    # its own workspace root), not this workspace's.
    pathDepDirs = pkg:
      map (
        d: let
          baseDir =
            if d.pathBase or "package" == "workspace"
            then pkg.wsRelDir
            else pkg.relDir;
        in
          normalizeRel (
            if baseDir == ""
            then d.path
            else "${baseDir}/${d.path}"
          )
      )
      (filter (d: d.path != null) pkg.deps);

    memberDirSet = listToAttrs (map (m: {
        name = m.relDir;
        value = true;
      })
      members);

    # For an external package using workspace inheritance, its workspace
    # root is the nearest ancestor (within src) whose manifest has a
    # [workspace] table; fall back to the package's own manifest.
    parentDir = d: let
      m = match "(.*)/[^/]*" d;
    in
      if m == null
      then ""
      else builtins.head m;

    wsRootFor = d: let
      mf = joinPath src (
        if d == ""
        then "Cargo.toml"
        else "${d}/Cargo.toml"
      );
      manifest =
        if pathExists mf
        then loadManifest mf
        else null;
    in
      if manifest != null && manifest ? workspace
      then {
        inherit manifest;
        dir = d;
      }
      else if d == ""
      then null
      else wsRootFor (parentDir d);

    walkExternal = seen: queue:
      if queue == []
      then seen
      else let
        d = builtins.head queue;
        rest = builtins.tail queue;
      in
        if memberDirSet ? ${d} || seen ? ${d}
        then walkExternal seen rest
        else let
          manifest = loadManifest (joinPath src "${d}/Cargo.toml");
          found = wsRootFor d;
          pkg = normalizePackage {
            dir = joinPath src d;
            relDir = d;
            wsRelDir =
              if found == null
              then d
              else found.dir;
            inherit manifest;
            workspaceManifest =
              if found == null
              then manifest
              else found.manifest;
          };
        in
          walkExternal (seen // {${d} = pkg;}) (rest ++ pathDepDirs pkg);

    pathPackages =
      builtins.attrValues
      (walkExternal {} (concatLists (map pathDepDirs members)));
  in {
    inherit rootManifest members pathPackages;
    byName = listToAttrs (map (m: {
      inherit (m) name;
      value = m;
    }) (members ++ pathPackages));
  };
in {
  inherit loadManifest loadWorkspace normalizePackage normalizeDeps normalizeRel snakeName joinPath globMatch;
}
