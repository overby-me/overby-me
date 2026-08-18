# Run: nix eval -f platform/nix/lib/lib/cargo/tests/resolve-real.nix --apply 'f: f {}'
#
# The `--apply` is because of the arguments below: without it nix prints the
# function rather than running the test, which reads like a pass.
#
# Resolves the real wclip and xz lockfiles against the committed snapshot
# index. Resolution itself verifies every crate checksum (lock vs index).
#
# The two real workspaces are arguments, so this file does not have to know
# where they live. The defaults are where they sit in the monorepo, which is
# what keeps the line above working from a checkout of it; the cargo-lib
# check passes flake inputs instead, which is what makes this run in a clone
# of this directory alone.
{
  wclipSrc ? ../../../../../../dev/wclip,
  xzSrc ? ../../../../../../safety/oxidized/xz,
}: let
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

  wclip = resolveProject wclipSrc ["rust-wclip"] false;
  xz = resolveProject xzSrc ["rust-xz"] false;
  xzDev = resolveProject xzSrc ["rust-xz"] true;

  xzNames = builtins.attrNames xz;
  has = nodes: name:
    builtins.any (id: nodes.${id}.pkg.name == name) (builtins.attrNames nodes);

  checks = {
    wclipNodes = builtins.sort (a: b: a < b) (builtins.attrNames wclip) == ["libc-0.2.186" "rust-wclip-0.1.0"];
    # libc default feature enables std
    wclipLibcFeatures = wclip."libc-0.2.186".features == ["default" "std"];
    wclipRootEdge = map (e: e.targetId) wclip."rust-wclip-0.1.0".edges == ["libc-0.2.186"];

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
