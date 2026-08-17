# The build every repo published out of the overby.me monorepo shares.
#
# Each of those repos is one filtered directory, and each needs the same
# things: a plain-nixpkgs build of its crate, a devshell carrying the gates
# the monorepo holds it to, and a formatter. Restating that in twenty-odd
# generated flakes meant twenty-odd copies of one hook list, so it lives here
# and each repo says only what is different about it.
# gitHooks is passed in rather than taken from the module's `inputs` argument,
# because that argument belongs to the flake importing this module, and the
# point of the module is that such a flake does not declare git-hooks itself.
{gitHooks}: {
  config,
  lib,
  ...
}: let
  inherit (lib) mkOption types;
  cfg = config.project;

  # Build inputs are named as strings rather than passed as packages, because
  # the generator that writes these flakes reads them out of a project list
  # and has no pkgs to resolve them against.
  resolve = pkgs: names: map (n: pkgs.${n}) names;
in {
  # What lets the module system recognise two copies of this as one module.
  #
  # A module has identity only if it is a path or carries a key. This one is
  # neither by nature: it is a function partially applied to gitHooks, so it
  # cannot be a path, and a bare function has no identity at all - the same
  # function value reaching evalModules twice declares every option in it
  # twice, and evaluation fails with "the option `project' is already
  # declared". That is what a second input exporting this module produces,
  # and it is why the integrations could not be selected through `?dir=`.
  # `_file` does not do this; it names a module for error messages only.
  key = "nix-workspace/module.nix";

  options.project = {
    name = mkOption {
      type = types.str;
      description = ''
        Fallback package name. A workspace root has no [package] in its
        Cargo.toml, and baseNameOf ./. is a store path, so the repo name has
        to be stated.
      '';
    };

    description = mkOption {
      type = types.str;
      default = "";
      description = ''
        One line about the project, becoming the package's meta.description.
        Stated here rather than alongside it as a flakelight option, so a
        published flake states everything it has to say in one attribute set.
      '';
    };

    root = mkOption {
      type = types.path;
      description = "The repo root, so the module can read its Cargo.toml.";
    };

    subdir = mkOption {
      type = types.str;
      default = "";
      description = ''
        Set when the crate is not at the repo root. A project whose
        Cargo.toml has a path dependency on a sibling is published as several
        directories, to keep that path correct, and its own crate is then one
        level down.
      '';
    };

    nativeBuildInputs = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "nixpkgs attribute names of build-time tools.";
    };

    buildInputs = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "nixpkgs attribute names of libraries linked against.";
    };

    doCheck = mkOption {
      type = types.bool;
      default = true;
      description = "Whether to run the crate's tests during the build.";
    };

    cargoTestFlags = mkOption {
      type = types.listOf types.str;
      default = [];
      description = "Extra arguments for the test phase, e.g. --skip.";
    };

    env = mkOption {
      type = types.functionTo (types.attrsOf types.str);
      default = _: {};
      description = ''
        Build-time environment, as a function of pkgs, for a crate whose
        build.rs needs something it cannot derive from its own sources.
      '';
    };

    toolchain = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Read rustc from the repo's own rust-toolchain.toml rather than taking
        nixpkgs'. A compiler plugin links against rustc's internals and needs
        the exact nightly it was written for. Requires the rust-overlay
        overlay, which the consuming flake supplies.
      '';
    };

    aliases = mkOption {
      type = types.attrsOf types.str;
      default = {};
      example = {
        gawk = "awk";
        sh = "bash";
      };
      description = ''
        Extra names for a binary this crate builds, as alias -> target.

        These tools are drop-in replacements, and the name they replace is
        usually not the one cargo produces: awk also answers to gawk, bash to
        sh, and a multicall binary answers to all thirty-four of PipeWire's
        tool names. The monorepo's own build has always added them; without
        this a published repo shipped only what Cargo.toml declares as a
        [[bin]], which for a multicall crate is a binary with none of the
        names it dispatches on and so of no use to anyone who cloned it.

        The target is a binary cargo built, not another alias: linking an
        alias to an alias works but leaves the published repo depending on
        an ordering this does not promise.
      '';
    };

    hooks = mkOption {
      type = types.attrsOf types.anything;
      default = {
        rustfmt.enable = true;
        clippy.enable = true;
        typos.enable = true;
        ripsecrets.enable = true;
        alejandra.enable = true;
        deadnix.enable = true;
        statix.enable = true;
      };
      description = ''
        The pre-commit hooks installed by `nix develop`, so a contributor who
        has only this repo is held to what the monorepo holds it to.
      '';
    };
  };

  config = {
    pname = cfg.name;
    formatter = pkgs: pkgs.alejandra;
    description = lib.mkIf (cfg.description != "") cfg.description;

    # flakelight turns this into packages.default, a check and an overlay, so
    # `nix flake check` builds it and a published repo's CI needs nothing
    # beyond `nix flake check`.
    package = {
      pkgs,
      rustPlatform,
      ...
    }: let
      cargoToml = builtins.fromTOML (builtins.readFile "${cfg.root}/${cfg.subdir}/Cargo.toml");
      cargoPackage = cargoToml.package or {};

      platform =
        if cfg.toolchain
        then let
          toolchain = pkgs.rust-bin.fromRustupToolchainFile "${cfg.root}/rust-toolchain.toml";
        in
          pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          }
        else rustPlatform;
    in
      platform.buildRustPackage ({
          pname = cargoPackage.name or cfg.name;
          version = cargoPackage.version or "0.1.0";
          src = cfg.root;
          cargoLock = {
            lockFile = "${cfg.root}/${cfg.subdir}/Cargo.lock";
            allowBuiltinFetchGit = true;
          };

          nativeBuildInputs = resolve pkgs cfg.nativeBuildInputs;
          buildInputs = resolve pkgs cfg.buildInputs;
          inherit (cfg) doCheck;

          meta = {
            inherit (config) description;
            mainProgram = cargoPackage.name or cfg.name;
          };
        }
        // (cfg.env pkgs)
        // lib.optionalAttrs (cfg.cargoTestFlags != []) {
          inherit (cfg) cargoTestFlags;
        }
        # Failing loudly on a target that is not there, because the monorepo
        # learned that the other way round: three packages symlinked every
        # one of their tool names to a binary that did not exist, and nothing
        # noticed until nixpkgs' noBrokenSymlinks turned it into an error.
        // lib.optionalAttrs (cfg.aliases != {}) {
          postInstall = lib.concatStringsSep "\n" (
            lib.mapAttrsToList (alias: target: ''
              test -e "$out/bin/${target}" \
                || { echo "aliases.${alias} names ${target}, which this crate does not build" >&2; exit 1; }
              ln -s "$out/bin/${target}" "$out/bin/${alias}"
            '')
            cfg.aliases
          );
        }
        # Both are needed and do different jobs: cargoRoot tells the setup
        # hook where Cargo.lock and the vendor config belong, buildAndTestSubdir
        # tells the build hook where to run cargo.
        // lib.optionalAttrs (cfg.subdir != "") {
          cargoRoot = cfg.subdir;
          buildAndTestSubdir = cfg.subdir;
        });

    devShell = let
      hooksFor = pkgs:
        gitHooks.lib.${pkgs.stdenv.hostPlatform.system}.run {
          src = cfg.root;
          package = pkgs.prek;
          inherit (cfg) hooks;
        };

      rustFor = pkgs:
        if cfg.toolchain
        then [(pkgs.rust-bin.fromRustupToolchainFile "${cfg.root}/rust-toolchain.toml") pkgs.rust-analyzer]
        else (with pkgs; [cargo rustc rust-analyzer clippy rustfmt]);
    in {
      # inputsFrom the package rather than restating its inputs: it brings in
      # whatever the build needs, including the setup hooks that put
      # pkg-config and friends in the right place, and it cannot fall out of
      # step with the package the way a second list would.
      inputsFrom = pkgs: [pkgs.${cfg.name}];
      packages = pkgs: (rustFor pkgs) ++ (hooksFor pkgs).enabledPackages;
      shellHook = pkgs: (hooksFor pkgs).shellHook;
      env = pkgs: cfg.env pkgs;
    };
  };
}
