# Run: nix eval -f platform/nix/lib/lib/cargo/tests/manifest.nix --apply 'f: f {}'
#
# The `--apply` is because of the arguments below: without it nix prints the
# function rather than running the test, which reads like a pass.
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
  manifest = import ../lib/manifest.nix;

  wclip = manifest.loadWorkspace wclipSrc;
  xz = manifest.loadWorkspace xzSrc;

  wclipPkg = wclip.byName.rust-wclip;
  xzPkg = xz.byName.rust-xz;

  wclipLibc = builtins.head wclipPkg.deps;
  xzDepsByName = builtins.listToAttrs (map (d: {
      inherit (d) name;
      value = d;
    })
    xzPkg.deps);

  checks = {
    wclipSingleMember = builtins.length wclip.members == 1;
    wclipName = wclipPkg.name == "rust-wclip";
    wclipEdition = wclipPkg.edition == "2024";
    wclipNoLib = wclipPkg.lib == null;
    wclipBin =
      wclipPkg.bins
      == [
        {
          name = "wclip";
          path = "src/main.rs";
          requiredFeatures = [];
        }
      ];
    wclipNoBuildScript = !wclipPkg.hasBuildScript;
    wclipDeps = builtins.length wclipPkg.deps == 1;
    wclipLibcDep =
      wclipLibc.name
      == "libc"
      && wclipLibc.req == "0.2"
      && wclipLibc.kind == "normal"
      && wclipLibc.defaultFeatures
      && !wclipLibc.optional;

    xzLib = xzPkg.lib.name == "rust_xz" && xzPkg.lib.path == "src/lib.rs" && !xzPkg.lib.procMacro;
    # Explicit [[bin]] plus the auto-discovered src/bin/rust-xz-fuzz.rs.
    xzBin =
      xzPkg.bins
      == [
        {
          name = "xz";
          path = "src/main.rs";
          requiredFeatures = [];
        }
        {
          name = "rust-xz-fuzz";
          path = "src/bin/rust-xz-fuzz.rs";
          requiredFeatures = [];
        }
      ];
    xzDepCount = builtins.length xzPkg.deps == 3;
    xzLiblzma = xzDepsByName.liblzma.kind == "normal" && xzDepsByName.liblzma.req == "0.4";
    xzCriterion =
      xzDepsByName.criterion.kind
      == "dev"
      && xzDepsByName.criterion.req == "0.5"
      && !xzDepsByName.criterion.defaultFeatures;
    xzProptest = xzDepsByName.proptest.kind == "dev" && xzDepsByName.proptest.req == "1";

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
