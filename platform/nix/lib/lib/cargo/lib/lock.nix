# Cargo.lock parsing (format v3/v4). Pure builtins.
#
# The lock provides the resolved package graph: exact versions, sources,
# checksums, and per-package resolved dependency lists. It does NOT contain
# dependency kinds, feature tables, optionality, or cfg gates; those come
# from the registry index (index.nix) and workspace manifests (manifest.nix).
let
  inherit
    (builtins)
    elemAt
    filter
    foldl'
    fromTOML
    head
    length
    listToAttrs
    match
    ;

  semver = import ./semver.nix;

  # "registry+https://..." | "sparse+https://..." | "git+https://...?rev=x#sha" | null
  parseSource = source:
    if source == null
    then {type = "path";}
    else let
      mReg = match "(registry|sparse)\\+(.*)" source;
      mGit = match "git\\+([^?#]*)(\\?[^#]*)?(#(.*))?" source;
    in
      if mReg != null
      then {
        type = "registry";
        url = elemAt mReg 1;
        cratesIo =
          elemAt mReg 1
          == "https://github.com/rust-lang/crates.io-index"
          || elemAt mReg 1 == "https://index.crates.io/";
      }
      else if mGit != null
      then {
        type = "git";
        url = head mGit;
        # Query string carries the branch/tag/rev request; the fragment is
        # the resolved commit, which is what matters for fetching.
        rev = elemAt mGit 3;
      }
      else throw "cargo lock: unrecognized source: ${source}";

  pkgId = p: "${p.name}-${p.version}";

  # Parse a resolved dependency string: "name", "name version",
  # or "name version (source)".
  parseDepString = s: let
    m = match "([A-Za-z0-9_-]+)( ([0-9][^ ]*))?( \\((.*)\\))?" s;
  in
    if m == null
    then throw "cargo lock: unparsable dependency string: ${s}"
    else {
      name = head m;
      version = elemAt m 2; # null if absent
      source = elemAt m 4; # null if absent
    };

  parseLock = lockText: let
    toml = fromTOML lockText;
    version = toml.version or 1;
    packages =
      map (p: {
        inherit (p) name version;
        source = p.source or null;
        sourceInfo = parseSource (p.source or null);
        checksum = p.checksum or null;
        depStrings = p.dependencies or [];
        id = pkgId p;
      })
      (toml.package or []);

    byId = listToAttrs (map (p: {
        name = p.id;
        value = p;
      })
      packages);

    byName =
      foldl' (
        acc: p:
          acc // {${p.name} = (acc.${p.name} or []) ++ [p];}
      ) {}
      packages;

    # Resolve one dependency string of pkg to a package id.
    resolveDepString = s: let
      d = parseDepString s;
      cands0 = byName.${d.name} or [];
      cands1 =
        if d.version == null
        then cands0
        else filter (p: p.version == d.version) cands0;
      cands =
        if d.source == null
        then cands1
        else filter (p: p.source == d.source) cands1;
    in
      if length cands == 1
      then (head cands).id
      else if cands == []
      then throw "cargo lock: dependency ${s} matches no locked package"
      else throw "cargo lock: dependency ${s} is ambiguous (${toString (length cands)} candidates)";

    # id -> list of resolved dependency package ids
    resolvedDeps = listToAttrs (map (p: {
        name = p.id;
        value = map resolveDepString p.depStrings;
      })
      packages);
  in {
    inherit version packages byId byName resolvedDeps;
    workspaceMembers = filter (p: p.sourceInfo.type == "path") packages;
    registryPackages = filter (p: p.sourceInfo.type == "registry") packages;
  };

  # Among a package's resolved deps, find the one matching a metadata
  # dependency record (crate name + semver req). Returns an id or null.
  # `depIds` are the ids from resolvedDeps, `lock` the parseLock result.
  findDep = lock: depIds: crateName: req: let
    cands = filter (
      p:
        p.name
        == crateName
        && (req == null || semver.matches req p.version)
    ) (map (id: lock.byId.${id}) depIds);
    best =
      foldl' (
        a: p:
          if a == null || semver.cmp p.version a.version > 0
          then p
          else a
      )
      null
      cands;
  in
    if best == null
    then null
    else best.id;
in {
  inherit parseLock parseSource parseDepString findDep pkgId;
}
