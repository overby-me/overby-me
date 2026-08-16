# The flake a project's outputs are assembled into.
#
# This is what flakelight did for us, kept because the module system is worth
# having and rewritten because the rest was not ours. A published repo now
# depends on nix-project and nixpkgs, and on nothing that exists to bridge
# between them.
#
# What is deliberately absent: nixDir, which platform/nix/project/outputs.nix
# already replaced with the same rule projects follow, and the multi-formatter
# builder, because a formatter here is one package and a shell script that
# dispatches on file extension is not worth carrying.
#
#   mkFlake { inherit nixpkgs; } ./. { ... module ... }
{nixpkgs}: src: module: let
  inherit (nixpkgs) lib;

  inherit (lib) evalModules genAttrs mapAttrs' mkOption nameValuePair optionalAttrs types;

  # A value that may be given as `pkgs: v` or as `v` itself, which is what
  # lets a module write `packages = pkgs: [...]` in one place and a literal
  # in another.
  applyPkgs = pkgs: v:
    if lib.isFunction v
    then v pkgs
    else v;

  evaluated = evalModules {
    modules = [
      {
        options = {
          systems = mkOption {
            type = types.listOf types.str;
            default = ["x86_64-linux"];
          };

          description = mkOption {
            type = types.nullOr types.str;
            default = null;
          };

          # The package this flake is for, as a callPackage-style definition.
          # It becomes packages.default, packages.<pname>, an overlay entry
          # under both names, and a check.
          package = mkOption {
            type = types.nullOr (types.functionTo types.raw);
            default = null;
          };

          pname = mkOption {
            type = types.nullOr types.str;
            default = null;
          };

          # Each attribute is a value or a function of pkgs: inputsFrom,
          # packages, shellHook, env.
          devShell = mkOption {
            type = types.attrsOf types.raw;
            default = {};
          };

          formatter = mkOption {
            type = types.nullOr (types.functionTo types.package);
            default = null;
          };

          withOverlays = mkOption {
            type = types.listOf types.raw;
            default = [];
          };

          nixpkgs.config = mkOption {
            type = types.attrsOf types.raw;
            default = {};
          };

          # Anything a module wants to put in the flake untouched.
          outputs = mkOption {
            type = types.attrsOf types.raw;
            default = {};
          };

          # Makes the resulting flake callable, which is how a consuming repo
          # says `inputs.project ./. { ... }` instead of importing a module.
          functor = mkOption {
            type = types.nullOr (types.functionTo types.raw);
            default = null;
          };
        };
      }
      module
    ];
  };

  cfg = evaluated.config;

  # The package, under both names, so a devshell can reach it as
  # `pkgs.<pname>` the way it could before.
  packageOverlay = _final: prev:
    optionalAttrs (cfg.package != null) (
      let
        drv = prev.callPackage cfg.package {};
      in
        {default = drv;} // optionalAttrs (cfg.pname != null) {${cfg.pname} = drv;}
    );

  pkgsFor = system:
    import nixpkgs {
      inherit system;
      inherit (cfg.nixpkgs) config;
      overlays = cfg.withOverlays ++ [packageOverlay];
    };

  eachSystem = f: genAttrs cfg.systems (system: f (pkgsFor system));

  packagesFor = pkgs:
    optionalAttrs (cfg.package != null) (
      {inherit (pkgs) default;}
      // optionalAttrs (cfg.pname != null) {${cfg.pname} = pkgs.${cfg.pname};}
    );

  # Anything besides the three known keys is environment: mkShell puts an
  # unrecognised attribute into the shell as a variable, which is how a
  # project whose build.rs needs a path gets one in the devshell too.
  devShellFor = pkgs:
    pkgs.mkShell (
      {
        inputsFrom = applyPkgs pkgs (cfg.devShell.inputsFrom or []);
        packages = applyPkgs pkgs (cfg.devShell.packages or []);
        shellHook = applyPkgs pkgs (cfg.devShell.shellHook or "");
      }
      // applyPkgs pkgs (cfg.devShell.env or {})
    );

  # Formatting is a check so that `nix flake check` fails on an unformatted
  # tree, which is the whole reason a generated flake is formatted before it
  # is written.
  formattingCheck = pkgs:
    pkgs.runCommand "check-formatting" {} ''
      cp -r --no-preserve=mode ${src} ./src
      cd ./src
      ${lib.getExe (cfg.formatter pkgs)} . >/dev/null
      if ! ${pkgs.diffutils}/bin/diff -qr ${src} . >/tmp/formatting-diff; then
        echo "not formatted:" >&2
        sed 's/Files .* and \(.*\) differ/  \1/' /tmp/formatting-diff >&2
        exit 1
      fi
      touch $out
    '';

  checksFor = pkgs:
    mapAttrs' (n: nameValuePair "packages-${n}") (packagesFor pkgs)
    // optionalAttrs (cfg.formatter != null) {formatting = formattingCheck pkgs;};
in
  {
    packages = eachSystem packagesFor;
    checks = eachSystem checksFor;
    devShells = eachSystem (pkgs: {default = devShellFor pkgs;});
  }
  // optionalAttrs (cfg.formatter != null) {
    formatter = eachSystem cfg.formatter;
  }
  // optionalAttrs (cfg.package != null) {
    overlays.default = final: prev: removeAttrs (packageOverlay final prev) ["default"];
  }
  // cfg.outputs
  // lib.optionalAttrs (cfg.functor != null) {__functor = cfg.functor;}
