# Minimal .buckconfig parser (INI-like). builtins only.
#
# Returns { sections; cells; }, where `cells` maps a cell name to its path
# relative to the project root. no_prelude's config is just:
#   [cells]
#   root = .
#   toolchains = toolchains
let
  inherit (builtins) substring stringLength filter isString;

  charAt = s: i: substring i 1 s;
  trim = s: let
    L = stringLength s;
    isWs = c: c == " " || c == "\t" || c == "\r";
    l = let
      go = i:
        if i < L && isWs (charAt s i)
        then go (i + 1)
        else i;
    in
      go 0;
    r = let
      go = i:
        if i > l && isWs (charAt s (i - 1))
        then go (i - 1)
        else i;
    in
      go L;
  in
    substring l (r - l) s;

  splitLines = s: filter isString (builtins.split "\n" s);

  # Strip a trailing/inline comment starting with # or ;.
  stripComment = line: let
    L = stringLength line;
    go = i:
      if i >= L
      then line
      else let
        c = charAt line i;
      in
        if c == "#" || c == ";"
        then substring 0 i line
        else go (i + 1);
  in
    go 0;

  indexOfChar = c: s: let
    L = stringLength s;
    go = i:
      if i >= L
      then -1
      else if charAt s i == c
      then i
      else go (i + 1);
  in
    go 0;

  parse = text: let
    lines = map (l: trim (stripComment l)) (splitLines text);
    step = acc: line:
      if line == ""
      then acc
      else if charAt line 0 == "["
      then let
        close = indexOfChar "]" line;
        section = substring 1 (close - 1) line;
      in
        acc
        // {
          current = section;
          sections = acc.sections // {${section} = acc.sections.${section} or {};};
        }
      else let
        eq = indexOfChar "=" line;
      in
        if eq < 0 || acc.current == null
        then acc
        else let
          key = trim (substring 0 eq line);
          value = trim (substring (eq + 1) (stringLength line - eq - 1) line);
        in
          acc
          // {
            sections =
              acc.sections
              // {
                ${acc.current} = (acc.sections.${acc.current} or {}) // {${key} = value;};
              };
          };
    result =
      builtins.foldl' step {
        current = null;
        sections = {};
      }
      lines;
  in {
    inherit (result) sections;
    cells = result.sections.cells or {};
  };
in {
  inherit parse;
}
