# Run: nix eval -f platform/nix/lib/lib/cargo/tests/lock.nix
let
  lock = import ../lib/lock.nix;

  gitSource = lock.parseSource "git+https://github.com/example/repo?rev=abc#deadbeef";

  checks = {
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
