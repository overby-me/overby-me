# Registry index lookup. Pure builtins.
#
# Reads crate metadata from a checkout of a registry index: either the full
# crates.io index (github:rust-lang/crates.io-index) or a snapshot mini-index
# produced by tools/snapshot-index.nu. Both use the same layout: one file per
# crate (1/, 2/, 3/<c>/, <ab>/<cd>/<name>), one JSON object per line per
# version. The sparse index serves identical lines over HTTP, which is what
# makes snapshotting trivial.
#
# Schema: https://doc.rust-lang.org/cargo/reference/registry-index.html
let
  inherit
    (builtins)
    filter
    fromJSON
    head
    isString
    length
    pathExists
    readFile
    replaceStrings
    split
    stringLength
    substring
    ;

  upper = ["A" "B" "C" "D" "E" "F" "G" "H" "I" "J" "K" "L" "M" "N" "O" "P" "Q" "R" "S" "T" "U" "V" "W" "X" "Y" "Z"];
  lower = ["a" "b" "c" "d" "e" "f" "g" "h" "i" "j" "k" "l" "m" "n" "o" "p" "q" "r" "s" "t" "u" "v" "w" "x" "y" "z"];
  toLower = replaceStrings upper lower;

  # Index directory layout for a crate name (always lowercased).
  relPath = name: let
    n = toLower name;
    len = stringLength n;
  in
    if len == 1
    then "1/${n}"
    else if len == 2
    then "2/${n}"
    else if len == 3
    then "3/${substring 0 1 n}/${n}"
    else "${substring 0 2 n}/${substring 2 2 n}/${n}";

  # Normalize one raw index dependency entry into the shared dep record
  # shape used throughout this library.
  normalizeDep = d: {
    # `name` is the rename when the dependency is renamed; `package` is the
    # real crate name in that case.
    inherit (d) name;
    package = let
      p = d.package or null;
    in
      if p == null
      then d.name
      else p;
    inherit (d) req;
    kind = let
      k = d.kind or null;
    in
      if k == null
      then "normal"
      else k;
    optional = d.optional or false;
    defaultFeatures = d.default_features or true;
    features = d.features or [];
    target = d.target or null;
    registry = d.registry or null;
    path = null;
    git = null;
    rev = null;
  };

  parseEntries = raw:
    map fromJSON (filter (l: isString l && l != "") (split "\n" raw));

  # Look up the metadata for name@version in an index checkout.
  lookup = indexDir: name: version: let
    p = relPath name;
    file = indexDir + "/${p}";
    entries = parseEntries (readFile file);
    found = filter (e: e.vers == version) entries;
    e = head found;
    schemaV = e.v or 1;
    features2 = e.features2 or {};
  in
    if !pathExists file
    then
      throw ''
        cargo index: no entry for crate ${name}: expected ${p} in ${toString indexDir}.
        If this is a snapshot index, re-run tools/snapshot-index.nu to include it.''
    else if found == []
    then
      throw ''
        cargo index: crate ${name} has no version ${version} in ${p} (${toString (length entries)} versions present).
        If this is a snapshot index, re-run tools/snapshot-index.nu after updating the lockfile.''
    else if schemaV > 2
    then throw "cargo index: entry ${name}@${version} uses schema v${toString schemaV}, which this library does not understand yet"
    else {
      inherit (e) name;
      version = e.vers;
      deps = map normalizeDep e.deps;
      # Schema v2 carries the extended feature syntax (dep:, ?/) in a
      # separate field for compatibility with older cargo; merge it.
      features = (e.features or {}) // features2;
      inherit (e) cksum;
      links = e.links or null;
      yanked = e.yanked or false;
    };
in {
  inherit relPath toLower lookup parseEntries;
}
