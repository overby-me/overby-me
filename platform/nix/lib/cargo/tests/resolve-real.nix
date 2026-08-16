# Run: nix eval -f platform/nix/lib/cargo/tests/resolve-real.nix
#
# Resolves the real wclip and xz lockfiles against the committed snapshot
# index. Resolution itself verifies every crate checksum (lock vs index).
let
  lockLib = import ../lib/lock.nix;
  cfgLib = import ../lib/cfg.nix;
  manifestLib = import ../lib/manifest.nix;
  resolveLib = import ../lib/resolve.nix;

  platform = cfgLib.platforms.x86_64-linux;
  indexDir = ../index;

  resolveProject = src: roots: includeDev:
    (resolveLib.resolve {
      lock = lockLib.parseLock (builtins.readFile (src + "/Cargo.lock"));
      inherit indexDir platform roots includeDev;
      workspace = manifestLib.loadWorkspace src;
    }).nodes;

  wclip = resolveProject ../../../../../dev/wclip ["wclip"] false;
  xz = resolveProject ../../../../../safety/oxidized/xz ["oxidized-xz"] false;
  xzDev = resolveProject ../../../../../safety/oxidized/xz ["oxidized-xz"] true;

  xzNames = builtins.attrNames xz;
  has = nodes: name:
    builtins.any (id: nodes.${id}.pkg.name == name) (builtins.attrNames nodes);

  checks = {
    wclipNodes = builtins.sort (a: b: a < b) (builtins.attrNames wclip) == ["libc-0.2.186" "wclip-0.1.0"];
    # libc default feature enables std
    wclipLibcFeatures = wclip."libc-0.2.186".features == ["default" "std"];
    wclipRootEdge = map (e: e.targetId) wclip."wclip-0.1.0".edges == ["libc-0.2.186"];

    xzHasLiblzma = has xz "liblzma" && has xz "liblzma-sys";
    # dev-deps must not be pulled in for a plain build
    xzNoDevDeps = !(has xz "criterion") && !(has xz "proptest") && !(has xz "regex");
    # but they appear when requested
    xzDevHasCriterion = has xzDev "criterion" && has xzDev "proptest";
    # far fewer than the 77 locked packages are needed for the bin build
    xzSmall = builtins.length xzNames < 15;
    xzDevLarge = builtins.length (builtins.attrNames xzDev) > 50;
  };

  failures = builtins.filter (n: !checks.${n}) (builtins.attrNames checks);
in
  if failures == []
  then "ok: ${toString (builtins.length (builtins.attrNames checks))} checks (wclip ${toString (builtins.length (builtins.attrNames wclip))} nodes, xz ${toString (builtins.length xzNames)} nodes, xz+dev ${toString (builtins.length (builtins.attrNames xzDev))} nodes)"
  else throw "resolve-real test failures: ${builtins.concatStringsSep ", " failures}"
