# cfg() expression parsing and evaluation for target-gated dependencies.
# Pure builtins.
#
# The registry index and manifests gate dependencies with either a full
# target triple ("x86_64-pc-windows-gnu") or a cfg expression
# ("cfg(all(unix, not(target_os = \"macos\")))"). This module parses and
# evaluates both against a platform description.
let
  inherit
    (builtins)
    all
    any
    elem
    elemAt
    head
    match
    substring
    tail
    ;

  throw' = builtins.throw;

  # Tokenizer. Tokens: { t = "ident" | "str" | "sym"; v = ...; }
  tokenize = s:
    if match "[[:space:]]*" s != null
    then []
    else let
      mIdent = match "[[:space:]]*([A-Za-z_][A-Za-z0-9_]*)(.*)" s;
      mStr = match "[[:space:]]*\"([^\"]*)\"(.*)" s;
      mSym = match "[[:space:]]*([(),=])(.*)" s;
    in
      if mIdent != null
      then
        [
          {
            t = "ident";
            v = head mIdent;
          }
        ]
        ++ tokenize (elemAt mIdent 1)
      else if mStr != null
      then
        [
          {
            t = "str";
            v = head mStr;
          }
        ]
        ++ tokenize (elemAt mStr 1)
      else if mSym != null
      then
        [
          {
            t = "sym";
            v = head mSym;
          }
        ]
        ++ tokenize (elemAt mSym 1)
      else throw' "cfg: tokenize error at: ${s}";

  isSym = tk: v: tk.t == "sym" && tk.v == v;

  expect = v: toks:
    if toks != [] && isSym (head toks) v
    then tail toks
    else throw' "cfg: expected ${v}";

  # Recursive descent parser. Each function returns { e or es, toks }.
  # expr := all(list) | any(list) | not(expr) | pred
  # pred := ident | ident = "string"
  parseExpr = toks:
    if toks == []
    then throw' "cfg: unexpected end of expression"
    else let
      tk = head toks;
      rest = tail toks;
    in
      if tk.t == "ident" && (tk.v == "all" || tk.v == "any") && rest != [] && isSym (head rest) "("
      then let
        args = parseList (tail rest);
      in {
        e = {
          op = tk.v;
          args = args.es;
        };
        inherit (args) toks;
      }
      else if tk.t == "ident" && tk.v == "not" && rest != [] && isSym (head rest) "("
      then let
        inner = parseExpr (tail rest);
        after = expect ")" inner.toks;
      in {
        e = {
          op = "not";
          arg = inner.e;
        };
        toks = after;
      }
      else if tk.t == "ident"
      then
        if rest != [] && isSym (head rest) "="
        then let
          sv = head (tail rest);
        in
          if sv.t != "str"
          then throw' "cfg: expected string after ="
          else {
            e = {
              op = "pred";
              name = tk.v;
              value = sv.v;
            };
            toks = tail (tail rest);
          }
        else {
          e = {
            op = "pred";
            name = tk.v;
            value = null;
          };
          toks = rest;
        }
      else throw' "cfg: parse error at token ${tk.v}";

  # Comma-separated expressions terminated by ")". Trailing comma allowed.
  parseList = toks:
    if toks != [] && isSym (head toks) ")"
    then {
      es = [];
      toks = tail toks;
    }
    else let
      first = parseExpr toks;
      nxt = first.toks;
    in
      if nxt == []
      then throw' "cfg: unterminated list"
      else if isSym (head nxt) ","
      then let
        more = parseList (tail nxt);
      in {
        es = [first.e] ++ more.es;
        inherit (more) toks;
      }
      else if isSym (head nxt) ")"
      then {
        es = [first.e];
        toks = tail nxt;
      }
      else throw' "cfg: expected , or ) in list";

  # Parse a full "cfg(...)" string into an expression tree.
  parseCfg = s: let
    m = match "cfg\\((.*)\\)[[:space:]]*" s;
  in
    if m == null
    then throw' "cfg: not a cfg() expression: ${s}"
    else let
      parsed = parseExpr (tokenize (head m));
    in
      if parsed.toks != []
      then throw' "cfg: trailing tokens in: ${s}"
      else parsed.e;

  # Platform description. See `platforms` below for the expected shape.
  evalPred = platform: name: value:
    if value == null
    then
      # Name-only predicates: unix/windows are family shorthands; everything
      # else (test, debug_assertions, miri, ...) is false in dependency
      # resolution.
      if name == "unix"
      then elem "unix" platform.target_family
      else if name == "windows"
      then elem "windows" platform.target_family
      else false
    else if name == "target_family"
    then elem value platform.target_family
    else if name == "target_feature"
    then elem value (platform.target_feature or [])
    else if name == "target_has_atomic"
    then elem value (platform.target_has_atomic or ["8" "16" "32" "64" "ptr"])
    else if
      elem name [
        "target_os"
        "target_arch"
        "target_env"
        "target_vendor"
        "target_pointer_width"
        "target_endian"
        "target_abi"
      ]
    then (platform.${name} or "") == value
    else false;

  evalCfg = platform: e:
    if e.op == "all"
    then all (evalCfg platform) e.args
    else if e.op == "any"
    then any (evalCfg platform) e.args
    else if e.op == "not"
    then !(evalCfg platform e.arg)
    else evalPred platform e.name e.value;

  # Does a dependency `target` field apply to this platform?
  # `target` is either "cfg(...)" or a literal triple.
  matchesTarget = platform: target:
    if substring 0 4 target == "cfg("
    then evalCfg platform (parseCfg target)
    else target == platform.triple;

  # Common platform descriptions, keyed by Nix system.
  platforms = {
    x86_64-linux = {
      triple = "x86_64-unknown-linux-gnu";
      target_os = "linux";
      target_arch = "x86_64";
      target_env = "gnu";
      target_vendor = "unknown";
      target_pointer_width = "64";
      target_endian = "little";
      target_abi = "";
      target_family = ["unix"];
      target_feature = ["fxsr" "sse" "sse2"];
      target_has_atomic = ["8" "16" "32" "64" "128" "ptr"];
    };
    aarch64-linux = {
      triple = "aarch64-unknown-linux-gnu";
      target_os = "linux";
      target_arch = "aarch64";
      target_env = "gnu";
      target_vendor = "unknown";
      target_pointer_width = "64";
      target_endian = "little";
      target_abi = "";
      target_family = ["unix"];
      target_feature = ["neon"];
      target_has_atomic = ["8" "16" "32" "64" "128" "ptr"];
    };
    x86_64-darwin = {
      triple = "x86_64-apple-darwin";
      target_os = "macos";
      target_arch = "x86_64";
      target_env = "";
      target_vendor = "apple";
      target_pointer_width = "64";
      target_endian = "little";
      target_abi = "";
      target_family = ["unix"];
      target_feature = ["fxsr" "sse" "sse2" "sse3" "ssse3"];
      target_has_atomic = ["8" "16" "32" "64" "128" "ptr"];
    };
    aarch64-darwin = {
      triple = "aarch64-apple-darwin";
      target_os = "macos";
      target_arch = "aarch64";
      target_env = "";
      target_vendor = "apple";
      target_pointer_width = "64";
      target_endian = "little";
      target_abi = "";
      target_family = ["unix"];
      target_feature = ["neon"];
      target_has_atomic = ["8" "16" "32" "64" "128" "ptr"];
    };
  };

  platformFromSystem = system:
    platforms.${system} or (throw' "cfg: no platform description for system ${system}");
in {
  inherit parseCfg evalCfg matchesTarget platforms platformFromSystem;
}
