# Shared helper functions for YAML parsing and serialization.
# This module is imported by both fromYAML.nix and toYAML.nix.
lib: let
  inherit
    (lib)
    substring
    stringLength
    elemAt
    length
    filter
    replaceStrings
    isString
    isBool
    isInt
    isFloat
    match
    split
    tryEval
    fromJSON
    ;

  # ============================================================
  # String Utilities
  # ============================================================

  charAt = s: i: substring i 1 s;

  countIndent = s: let
    len = stringLength s;
    go = i:
      if i >= len
      then i
      else if charAt s i == " "
      then go (i + 1)
      else i;
  in
    go 0;

  trimLeft = s: let
    len = stringLength s;
    go = i:
      if i >= len
      then ""
      else if charAt s i == " " || charAt s i == "\t"
      then go (i + 1)
      else substring i (len - i) s;
  in
    go 0;

  trimRight = s: let
    len = stringLength s;
    go = i:
      if i <= 0
      then ""
      else if charAt s (i - 1) == " " || charAt s (i - 1) == "\t"
      then go (i - 1)
      else substring 0 i s;
  in
    go len;

  trim = s: trimLeft (trimRight s);

  splitLines = s: let
    parts = split "\n" s;
  in
    filter isString parts;

  hasInfix = infix: str: let
    ilen = stringLength infix;
    slen = stringLength str;
    go = i:
      if i + ilen > slen
      then false
      else if substring i ilen str == infix
      then true
      else go (i + 1);
  in
    if ilen == 0
    then true
    else go 0;

  hasPrefix = prefix: str:
    substring 0 (stringLength prefix) str == prefix;

  hasSuffix = suffix: str: let
    slen = stringLength str;
    suflen = stringLength suffix;
  in
    suflen <= slen && substring (slen - suflen) suflen str == suffix;

  # ============================================================
  # Scalar Constants & Utilities
  # ============================================================

  boolTrueValues = ["true" "True" "TRUE" "yes" "Yes" "YES" "on" "On" "ON"];
  boolFalseValues = ["false" "False" "FALSE" "no" "No" "NO" "off" "Off" "OFF"];
  nullValues = ["null" "Null" "NULL" "~" ""];

  elemOf = x: xs: let
    go = i:
      if i >= length xs
      then false
      else if elemAt xs i == x
      then true
      else go (i + 1);
  in
    go 0;

  tryParseNumber = s: let
    isIntStr = match "[-+]?[0-9]+" s != null;
    isFloatStr =
      match "[-+]?([0-9]+\\.[0-9]*|[0-9]*\\.[0-9]+)([eE][-+]?[0-9]+)?" s
      != null
      || match "[-+]?[0-9]+[eE][-+]?[0-9]+" s != null;
    isHexStr = match "0x[0-9a-fA-F]+" s != null;
    isOctStr = match "0o[0-7]+" s != null;
    isSpecialFloat =
      elemOf s [".inf" ".Inf" ".INF" "+.inf" "+.Inf" "+.INF" "-.inf" "-.Inf" "-.INF" ".nan" ".NaN" ".NAN"];
    # Normalize for fromJSON: strip leading +
    normalized =
      if stringLength s > 0 && charAt s 0 == "+"
      then substring 1 (stringLength s - 1) s
      else s;
    parsed = tryEval (fromJSON normalized);
  in
    if isSpecialFloat
    then {
      success = true;
      value = s;
    } # Keep as string since Nix has no inf/nan
    else if isHexStr || isOctStr
    then let
      p = tryEval (fromJSON s);
    in
      if p.success
      then {
        success = true;
        inherit (p) value;
      }
      else {success = false;}
    else if isIntStr || isFloatStr
    then
      if parsed.success
      then {
        success = true;
        inherit (parsed) value;
      }
      else {success = false;}
    else {success = false;};

  # ============================================================
  # Quoted String Handling
  # ============================================================

  unescapeDoubleQuoted = s:
    replaceStrings
    ["\\\\" "\\\"" "\\n" "\\t" "\\r" "\\/" "\\0" "\\a" "\\b" "\\e" "\\ "]
    ["\\" "\"" "\n" "\t" "\r" "/" "" "" "" "" " "]
    s;

  unescapeSingleQuoted = s:
    replaceStrings ["''"] ["'"] s;

  parseQuotedScalar = s: let
    len = stringLength s;
    quote = charAt s 0;
    inner = substring 1 (len - 2) s;
  in
    if quote == "\""
    then unescapeDoubleQuoted inner
    else unescapeSingleQuoted inner;

  isQuoted = s: let
    len = stringLength s;
  in
    len
    >= 2
    && (
      (charAt s 0 == "\"" && charAt s (len - 1) == "\"")
      || (charAt s 0 == "'" && charAt s (len - 1) == "'")
    );

  escapeDoubleQuoted = s:
    replaceStrings
    ["\\" "\"" "\n" "\t" "\r"]
    ["\\\\" "\\\"" "\\n" "\\t" "\\r"]
    s;

  # ============================================================
  # Scalar Parsing
  # ============================================================

  parseScalar = s: let
    trimmed = trim s;
  in
    if elemOf trimmed nullValues
    then null
    else if elemOf trimmed boolTrueValues
    then true
    else if elemOf trimmed boolFalseValues
    then false
    else if isQuoted trimmed
    then parseQuotedScalar trimmed
    else let
      num = tryParseNumber trimmed;
    in
      if num.success
      then num.value
      else trimmed;

  # ============================================================
  # Comment Stripping
  # ============================================================

  stripComment = line: let
    len = stringLength line;
    go = i: inSingle: inDouble:
      if i >= len
      then line
      else let
        c = charAt line i;
      in
        if inDouble
        then
          if c == "\\"
          then go (i + 2) inSingle inDouble
          else if c == "\""
          then go (i + 1) inSingle false
          else go (i + 1) inSingle inDouble
        else if inSingle
        then
          if c == "'"
          then
            if i + 1 < len && charAt line (i + 1) == "'"
            then go (i + 2) true inDouble
            else go (i + 1) false inDouble
          else go (i + 1) inSingle inDouble
        else if c == "\""
        then go (i + 1) inSingle true
        else if c == "'"
        then go (i + 1) true inDouble
        else if c == "#"
        then
          if i == 0 || charAt line (i - 1) == " " || charAt line (i - 1) == "\t"
          then trimRight (substring 0 i line)
          else go (i + 1) inSingle inDouble
        else go (i + 1) inSingle inDouble;
  in
    go 0 false false;

  # ============================================================
  # Scalar Type Detection
  # ============================================================

  isScalar = val:
    (val == null) || isBool val || isInt val || isFloat val || isString val;
in {
  inherit
    charAt
    countIndent
    trimLeft
    trimRight
    trim
    splitLines
    hasInfix
    hasPrefix
    hasSuffix
    boolTrueValues
    boolFalseValues
    nullValues
    elemOf
    tryParseNumber
    unescapeDoubleQuoted
    unescapeSingleQuoted
    parseQuotedScalar
    isQuoted
    escapeDoubleQuoted
    parseScalar
    stripComment
    isScalar
    ;
}
