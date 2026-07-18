# Run: nix eval -f nix/cargo/tests/manifest.nix
let
  manifest = import ../lib/manifest.nix;

  wclip = manifest.loadWorkspace ../../../../rust/wclip;
  xz = manifest.loadWorkspace ../../../../rust/xz;

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
