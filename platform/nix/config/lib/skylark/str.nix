# Shared string/char helpers for the skylark interpreter. builtins only, so
# unit tests run with a bare `nix eval -f platform/nix/config/lib/skylark/tests/<mod>.nix`.
let
  inherit (builtins) substring stringLength;

  charAt = s: i: substring i 1 s;

  # Single-ASCII-char classification via lexicographic comparison.
  isDigit = c: c >= "0" && c <= "9";
  isLower = c: c >= "a" && c <= "z";
  isUpper = c: c >= "A" && c <= "Z";
  isAlpha = c: isLower c || isUpper c || c == "_";
  isAlnum = c: isAlpha c || isDigit c;
  isHexDigit = c: isDigit c || (c >= "a" && c <= "f") || (c >= "A" && c <= "F");
  isSpace = c: c == " " || c == "\t";

  hasPrefix = prefix: str: substring 0 (stringLength prefix) str == prefix;

  hasSuffix = suffix: str: let
    sl = stringLength str;
    fl = stringLength suffix;
  in
    fl <= sl && substring (sl - fl) fl str == suffix;

  # Length-guarded slice: substring but clamps so callers can over-ask.
  slice = from: to: s: substring from (to - from) s;

  # Repeat a string n times.
  rep = s: n: let
    go = i: acc:
      if i <= 0
      then acc
      else go (i - 1) (acc + s);
  in
    go n "";

  # Split on a single-char delimiter into a list of strings (delimiter dropped).
  splitOn = delim: s: builtins.filter builtins.isString (builtins.split delim s);

  # First index of char c in s at or after `from`, or -1.
  indexOf = c: from: s: let
    len = stringLength s;
    go = i:
      if i >= len
      then -1
      else if charAt s i == c
      then i
      else go (i + 1);
  in
    go from;

  # Last non-empty component of a slash path (basename).
  baseName = p: let
    parts = splitOn "/" p;
    nonEmpty = builtins.filter (x: x != "") parts;
    n = builtins.length nonEmpty;
  in
    if n == 0
    then p
    else builtins.elemAt nonEmpty (n - 1);

  # Character code of a single char, and back. Covers ASCII we care about.
  # Built lazily as attrset lookups.
  asciiChars = "\t\n\r !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";
  # Codes for the printable range start at their real ASCII values; we only
  # need ord/chr for escape handling, which stays within ASCII here.

  trimAscii = s: let
    len = stringLength s;
    l = let
      go = i:
        if i < len && isSpace (charAt s i)
        then go (i + 1)
        else i;
    in
      go 0;
    r = let
      go = i:
        if i > l && isSpace (charAt s (i - 1))
        then go (i - 1)
        else i;
    in
      go len;
  in
    substring l (r - l) s;
in {
  inherit
    charAt
    isDigit
    isLower
    isUpper
    isAlpha
    isAlnum
    isHexDigit
    isSpace
    hasPrefix
    hasSuffix
    slice
    rep
    splitOn
    indexOf
    baseName
    trimAscii
    asciiChars
    ;
}
