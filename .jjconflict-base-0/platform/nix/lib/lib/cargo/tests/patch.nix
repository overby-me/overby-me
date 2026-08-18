# Run: nix eval -f platform/nix/lib/lib/cargo/tests/patch.nix
#
# A `[patch.crates-io]` override redirecting `mycrate` to a local path makes
# it a path package in the lock (no source). loadWorkspace must discover the
# patched directory, and resolve must build the graph against it rather than
# throwing "path package but not a workspace member".
let
  manifestLib = import ../lib/manifest.nix;
  lockLib = import ../lib/lock.nix;
  cfgLib = import ../lib/cfg.nix;
  resolveLib = import ../lib/resolve.nix;

  ws = manifestLib.loadWorkspace ./fixtures/patch-ws;
  lock = lockLib.parseLock (builtins.readFile ./fixtures/patch-ws/Cargo.lock);

  inherit
    (resolveLib.resolve {
      inherit lock;
      indexDir = ./fixtures/index; # unused: no registry packages in this lock
      platform = cfgLib.platforms.x86_64-linux;
      workspace = ws;
      roots = ["patch-root"];
    })
    nodes
    ;

  checks = {
    patchDiscovered = ws.byName ? mycrate;
    patchPath = ws.byName.mycrate.relDir == "vendor/mycrate";
    patchVersion = ws.byName.mycrate.version == "1.0.0";
    resolvedNode = nodes ? "mycrate-1.0.0";
    rootEdge = map (e: e.targetId) nodes."patch-root-0.1.0".edges == ["mycrate-1.0.0"];
    patchedFromLocal = nodes."mycrate-1.0.0".isWorkspaceMember;
  };
  failures = builtins.filter (n: !checks.${n}) (builtins.attrNames checks);
in
  if failures == []
  then "ok: ${toString (builtins.length (builtins.attrNames checks))} checks"
  else throw "patch test failures: ${builtins.concatStringsSep ", " failures}"
