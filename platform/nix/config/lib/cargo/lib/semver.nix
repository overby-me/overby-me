# Cargo semver requirement parsing and matching. Pure builtins.
#
# Implements the comparator semantics of the Rust `semver` crate as used by
# cargo: caret (default), tilde, wildcard, exact and range comparators,
# comma-separated conjunctions, and the pre-release opt-in rule.
# Reference: https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html
let
  inherit
    (builtins)
    all
    any
    elemAt
    filter
    fromJSON
    head
    isInt
    isString
    length
    match
    replaceStrings
    split
    tail
    ;

  toInt = s:
    if match "[0-9]+" s != null
    then
      fromJSON (
        if match "0[0-9]+" s != null
        then throw "leading zero in version component: ${s}"
        else s
      )
    else throw "invalid version component: ${s}";

  at = xs: i:
    if i < length xs
    then elemAt xs i
    else null;

  # Parse "1.2.*-pre+build" into { major, minor, patch, pre, starred }.
  # Missing and wildcard components are null; starred records whether an
  # explicit wildcard appeared (distinguishes "1.2" from "1.2.*").
  parseVersion = s: let
    mBuild = match "([^+]+)\\+.*" s;
    noBuild =
      if mBuild == null
      then s
      else head mBuild;
    mPre = match "([0-9xX*.]+)-(.*)" noBuild;
    core =
      if mPre == null
      then noBuild
      else head mPre;
    preStr =
      if mPre == null
      then ""
      else elemAt mPre 1;
    rawParts = filter isString (split "\\." core);
    isStar = p: p == "*" || p == "x" || p == "X";
    parts = map (p:
      if isStar p
      then null
      else toInt p)
    rawParts;
    preIds =
      if preStr == ""
      then []
      else
        map (
          i:
            if match "[0-9]+" i != null
            then toInt i
            else i
        ) (filter (i: isString i && i != "") (split "\\." preStr));
  in {
    major = at parts 0;
    minor = at parts 1;
    patch = at parts 2;
    pre = preIds;
    starred = any isStar rawParts;
  };

  # Fill missing components with 0 (keeps pre).
  fill = v: {
    major =
      if v.major == null
      then 0
      else v.major;
    minor =
      if v.minor == null
      then 0
      else v.minor;
    patch =
      if v.patch == null
      then 0
      else v.patch;
    inherit (v) pre;
  };

  mk = major: minor: patch: {
    inherit major minor patch;
    pre = [];
  };

  cmpInt = a: b:
    if a < b
    then -1
    else if a > b
    then 1
    else 0;

  # Pre-release identifier comparison: numeric < alphanumeric, numeric
  # compared numerically, alphanumeric lexically.
  cmpId = a: b:
    if isInt a && isInt b
    then cmpInt a b
    else if isInt a
    then -1
    else if isInt b
    then 1
    else if a < b
    then -1
    else if a > b
    then 1
    else 0;

  cmpIdList = a: b:
    if a == [] && b == []
    then 0
    else if a == []
    then -1
    else if b == []
    then 1
    else let
      c = cmpId (head a) (head b);
    in
      if c != 0
      then c
      else cmpIdList (tail a) (tail b);

  # Empty pre list means a release, which sorts above any pre-release.
  cmpPre = a: b:
    if a == [] && b == []
    then 0
    else if a == []
    then 1
    else if b == []
    then -1
    else cmpIdList a b;

  # Compare two filled versions.
  cmpVersion = a: b: let
    c1 = cmpInt a.major b.major;
    c2 = cmpInt a.minor b.minor;
    c3 = cmpInt a.patch b.patch;
  in
    if c1 != 0
    then c1
    else if c2 != 0
    then c2
    else if c3 != 0
    then c3
    else cmpPre a.pre b.pre;

  # Compare two version strings.
  cmp = a: b: cmpVersion (fill (parseVersion a)) (fill (parseVersion b));

  # Upper bound (exclusive) for caret: leftmost nonzero component rule.
  caretUpper = v:
    if v.major == null
    then null # "^*" degenerates to any
    else if v.major > 0
    then mk (v.major + 1) 0 0
    else if v.minor == null
    then mk 1 0 0 # ^0
    else if v.minor > 0
    then mk 0 (v.minor + 1) 0
    else if v.patch == null
    then mk 0 1 0 # ^0.0
    else mk 0 0 (v.patch + 1); # ^0.0.K

  # Upper bound (exclusive) when the last specified component is bumped:
  # used for wildcards, partial "=", partial "<=", partial ">".
  bumpUpper = v:
    if v.major == null
    then null
    else if v.minor == null
    then mk (v.major + 1) 0 0
    else mk v.major (v.minor + 1) 0;

  tildeUpper = v:
    if v.minor == null
    then mk (v.major + 1) 0 0
    else mk v.major (v.minor + 1) 0;

  parseComparator = c: let
    m = match "(>=|<=|>|<|\\^|~|=)(.*)" c;
    opRaw =
      if m == null
      then null
      else head m;
    rest =
      if m == null
      then c
      else elemAt m 1;
    v = parseVersion rest;
    op =
      if opRaw != null
      then opRaw
      else if v.starred
      then "wild"
      else "^";
  in {inherit op v;};

  # A requirement is a comma-separated conjunction of comparators.
  parseReq = req: let
    stripped = replaceStrings [" "] [""] req;
    parts = filter (s: isString s && s != "") (split "," stripped);
  in
    map parseComparator parts;

  partial = v: v.minor == null || v.patch == null;

  ge = ver: bound: cmpVersion ver bound >= 0;
  lt = ver: bound: bound == null || cmpVersion ver bound < 0;

  matchesCmp = ver: c: let
    inherit (c) v;
    lo = fill v;
  in
    if c.op == "^"
    then ge ver lo && lt ver (caretUpper v)
    else if c.op == "~"
    then ge ver lo && lt ver (tildeUpper v)
    else if c.op == "wild"
    then v.major == null || (ge ver lo && lt ver (bumpUpper v))
    else if c.op == "="
    then
      if partial v
      then ge ver lo && lt ver (bumpUpper v)
      else cmpVersion ver lo == 0
    else if c.op == ">="
    then ge ver lo
    else if c.op == ">"
    then
      if partial v
      then ge ver (bumpUpper v) # >I means >=(I+1).0.0, >I.J means >=I.(J+1).0
      else cmpVersion ver lo > 0
    else if c.op == "<"
    then cmpVersion ver lo < 0
    else if c.op == "<="
    then
      if partial v
      then lt ver (bumpUpper v) # <=I means <(I+1).0.0, <=I.J means <I.(J+1).0
      else cmpVersion ver lo <= 0
    else throw "semver: unknown comparator op ${c.op}";

  # Pre-release versions only match when some comparator names a pre-release
  # of the same major.minor.patch.
  preOk = ver: cmps:
    ver.pre
    == []
    || any (
      c:
        c.v.pre
        != []
        && c.v.major == ver.major
        && c.v.minor == ver.minor
        && c.v.patch == ver.patch
    )
    cmps;

  # Does version string verStr satisfy requirement string req?
  matches = req: verStr: let
    ver = fill (parseVersion verStr);
    cmps = parseReq req;
  in
    all (matchesCmp ver) cmps && preOk ver cmps;
in {
  inherit parseVersion fill cmpVersion cmp matches parseReq;
}
