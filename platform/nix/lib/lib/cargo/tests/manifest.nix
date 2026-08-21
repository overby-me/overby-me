# Run: nix eval -f platform/nix/lib/lib/cargo/tests/manifest.nix
let
  manifest = import ../lib/manifest.nix;

  checks = {
    snake = manifest.snakeName "foo-bar-baz" == "foo_bar_baz";

    globStar = manifest.globMatch "crates/*" "crates/foo";
    globStarNoCross = !(manifest.globMatch "crates/*" "crates/foo/bar");
    globStarStar = manifest.globMatch "**/bench" "a/b/bench";
    globQm = manifest.globMatch "cra?es/x" "crates/x";
    globEscape = !(manifest.globMatch "a.b" "axb");
  };

  failures = builtins.filter (n: !checks.${n}) (builtins.attrNames checks);
in
  if failures == []
  then "ok: ${toString (builtins.length (builtins.attrNames checks))} checks"
  else throw "manifest test failures: ${builtins.concatStringsSep ", " failures}"
