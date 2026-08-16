# Run: nix eval -f platform/nix/cargo/tests/lock.nix
let
  lock = import ../lib/lock.nix;

  wclip = lock.parseLock (builtins.readFile ../../../../../dev/wclip/Cargo.lock);
  xz = lock.parseLock (builtins.readFile ../../../../../safety/oxidized/xz/Cargo.lock);

  libc = wclip.byId."libc-0.2.186";
  member = wclip.byId."rust-wclip-0.1.0";

  gitSource = lock.parseSource "git+https://github.com/example/repo?rev=abc#deadbeef";

  checks = {
    wclipVersion = wclip.version == 4;
    wclipCount = builtins.length wclip.packages == 2;
    libcIsRegistry = libc.sourceInfo.type == "registry" && libc.sourceInfo.cratesIo;
    libcChecksum = builtins.stringLength libc.checksum == 64;
    memberIsPath = member.sourceInfo.type == "path";
    memberDeps = wclip.resolvedDeps."rust-wclip-0.1.0" == ["libc-0.2.186"];
    membersFound = map (p: p.name) wclip.workspaceMembers == ["rust-wclip"];
    findDepMatch = lock.findDep wclip ["libc-0.2.186"] "libc" "0.2" == "libc-0.2.186";
    findDepNoMatch = lock.findDep wclip ["libc-0.2.186"] "libc" "0.3" == null;
    findDepWrongName = lock.findDep wclip ["libc-0.2.186"] "serde" "1" == null;

    xzCount = builtins.length xz.packages == 77;
    xzRegistryCount = builtins.length xz.registryPackages == 76;
    xzMemberDeps = builtins.length xz.resolvedDeps."rust-xz-0.1.0" == 3;

    gitType = gitSource.type == "git";
    gitUrl = gitSource.url == "https://github.com/example/repo";
    gitRev = gitSource.rev == "deadbeef";

    depStringBare =
      lock.parseDepString "memchr"
      == {
        name = "memchr";
        version = null;
        source = null;
      };
    depStringVer =
      lock.parseDepString "syn 2.0.1"
      == {
        name = "syn";
        version = "2.0.1";
        source = null;
      };
    depStringFull =
      lock.parseDepString "syn 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)"
      == {
        name = "syn";
        version = "1.0.0";
        source = "registry+https://github.com/rust-lang/crates.io-index";
      };
  };

  failures = builtins.filter (n: !checks.${n}) (builtins.attrNames checks);
in
  if failures == []
  then "ok: ${toString (builtins.length (builtins.attrNames checks))} checks"
  else throw "lock test failures: ${builtins.concatStringsSep ", " failures}"
