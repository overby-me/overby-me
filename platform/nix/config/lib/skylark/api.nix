# Public API for the pure-Nix Starlark (Skylark) interpreter. builtins only.
#
# The interpreter is deliberately host-agnostic: `load()` is resolved by a
# caller-supplied `loadModule`, extra globals are injected via `extraGlobals`,
# and an opaque `world` accumulator is threaded through evaluation so a host
# (e.g. Buck2) can collect effects. See platform/nix/config/lib/buck2/PLAN.md.
let
  V = import ./values.nix;
  parser = import ./parser.nix;
  sk = import ./builtins.nix {inherit V;};

  stubLoad = _currentFile: _label:
    throw "skylark: load() requires a loadModule resolver (pass mkInterp { loadModule = ...; })";

  # Build an interpreter instance. Returns { evalModule; evalExpr; eval;
  # callValue; apply; baseEnv; }.
  mkInterp = {
    loadModule ? stubLoad,
    extraGlobals ? {},
  }:
    import ./eval.nix {inherit V sk loadModule extraGlobals;};

  base = mkInterp {};
in {
  inherit (parser) parse parseExpr;
  # Value model and standard library, for hosts building on top.
  values = V;
  stdlib = sk; # { globals; getMethod; }
  inherit mkInterp;

  # Evaluate a module source string. Returns { globals; world; }.
  exec = {
    src,
    currentFile ? "<string>",
    world ? null,
    extraGlobals ? {},
    loadModule ? stubLoad,
  }:
    (mkInterp {inherit loadModule extraGlobals;}).evalModule currentFile (parser.parse src) world;

  # Evaluate a single expression string in the base environment.
  evalExpr = src: base.eval (parser.parseExpr src);
}
