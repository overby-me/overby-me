# The flake a project's or a workspace's outputs are assembled into.
#
# This is what flakelight did for us, kept because the module system is worth
# having and rewritten because the rest was not ours. A published repo depends
# on nix-project and nixpkgs, and on nothing that exists to bridge between
# them.
#
# What is deliberately absent: nixDir, which platform/nix/project/outputs.nix
# already replaced with the same rule projects follow, and the multi-formatter
# builder, because a formatter here is one package and a shell script that
# dispatches on file extension is not worth carrying.
#
#   mkFlake { inherit nixpkgs; } ./. { ... module ... }
#
# Everything a module writes lands in `outputs`, which is what the flake
# becomes. The named options - packages, checks, nixosConfigurations and the
# rest - are conveniences that write into it, so a module can either use them
# or set `outputs.<anything>` directly when nothing fits.
{nixpkgs}: src: module: let
  inherit (nixpkgs) lib;

  inherit (lib) evalModules foldl genAttrs getFiles getValues isFunction mapAttrs mapAttrs' mkOption mkOptionType nameValuePair optionalAttrs recursiveUpdate showFiles showOption types;

  # Types a module can reach for. Same names and meanings as the ones our
  # modules were written against, because they are describing the same
  # things: a value that may be a function of the module arguments, a
  # nixpkgs overlay, a module.
  projectTypes = rec {
    nullable = t: types.nullOr t;

    # A value, or a function returning one. `optFunctionTo` is the shape a
    # per-system option takes: `packages = pkgs: [...]` or a literal.
    #
    # The coercion is declared from a synthetic non-function type rather than
    # from the element type, because nixpkgs refuses to coerce from a type
    # containing submodules and a devshell is one.
    optFunctionTo = elemType: let
      nonFunction = mkOptionType {
        name = "nonFunction";
        description = "non-function";
        descriptionClass = "noun";
        check = x: !isFunction x && elemType.check x;
        merge = lib.options.mergeOneOption;
      };
    in
      types.coercedTo nonFunction (x: _: x) (types.functionTo elemType);

    overlay = mkOptionType {
      name = "overlay";
      description = "nixpkgs overlay";
      descriptionClass = "noun";
      check = isFunction;
      merge = _: defs: lib.composeManyExtensions (getValues defs);
    };

    module = mkOptionType {
      name = "module";
      description = "module";
      descriptionClass = "noun";
      check = v: isFunction v || builtins.isAttrs v || builtins.isPath v;
      merge = _: defs: {imports = getValues defs;};
    };

    packageDef = mkOptionType {
      name = "packageDef";
      description = "package definition";
      descriptionClass = "noun";
      check = isFunction;
      merge = lib.options.mergeOneOption;
    };

    # A value that may be given as a function of the module arguments, which
    # is how a module writes `nixosModules.foo = {pkgs, ...}: ...` and still
    # gets our arguments rather than NixOS'.
    optCallWith = args: elemType:
      types.coercedTo function (x: x args) elemType;

    function = mkOptionType {
      name = "function";
      description = "function";
      descriptionClass = "noun";
      check = isFunction;
      merge = lib.options.mergeOneOption;
    };
  };

  # A devshell, described by options so that every shell has the same shape
  # whoever wrote it: a module that renames them all, or builds one for a
  # published repo, can rely on `packages` and `stdenv` and the rest being
  # there. Each is a value or a function of pkgs.
  devShellModule = {
    options = {
      inputsFrom = mkOption {
        type = projectTypes.optFunctionTo (types.listOf types.package);
        default = [];
      };
      packages = mkOption {
        type = projectTypes.optFunctionTo (types.listOf types.package);
        default = [];
      };
      shellHook = mkOption {
        type = projectTypes.optFunctionTo types.lines;
        default = "";
      };
      hardeningDisable = mkOption {
        type = types.listOf types.str;
        default = [];
      };
      env = mkOption {
        type = projectTypes.optFunctionTo (types.lazyAttrsOf types.str);
        default = {};
      };
      stdenv = mkOption {
        type = projectTypes.optFunctionTo types.package;
        default = pkgs: pkgs.stdenv;
      };
      # An already-built shell, used as-is.
      overrideShell = mkOption {
        type = types.nullOr types.package;
        default = null;
      };
    };
  };

  devShellType =
    projectTypes.optFunctionTo
    (types.coercedTo types.package (p: {overrideShell = p;})
      (types.submoduleWith {modules = [devShellModule];}));

  # Outputs merge by recursing into attribute sets, so two modules can each
  # contribute part of `outputs.checks.x86_64-linux` without either knowing
  # about the other. A conflict on a leaf is an error naming both files,
  # which is the whole reason not to merge with `//`.
  outputsType = mkOptionType {
    name = "outputs";
    description = "output values";
    descriptionClass = "noun";
    merge = loc: defs:
      if builtins.length defs == 1
      then (builtins.head defs).value
      else if builtins.all builtins.isAttrs (getValues defs)
      then (types.lazyAttrsOf outputsType).merge loc defs
      else
        throw (
          "The option `${showOption loc}' has conflicting definitions in "
          + showFiles (getFiles defs)
        );
  };

  evaluated = evalModules {
    specialArgs = {
      inherit src pkgsFor genSystems;
      project = {types = projectTypes;};
      # Named `flakelight` as well while our own modules still say so: the
      # types are the same shapes, and renaming every use is a separate job
      # from replacing what provides them.
      flakelight = {types = projectTypes;};
    };
    modules = [moduleArgsModule coreModule module];
  };

  cfg = evaluated.config;

  # The module arguments, made available to modules as an argument in their
  # own right, which is what an option type needs when it accepts a function
  # of them. Defined inside the evaluation rather than around it: computing
  # it outside would need the evaluation that needs it.
  moduleArgsModule = {config, ...} @ args: {
    _module.args.moduleArgs = args // config._module.args;
  };

  inherit (cfg) systems;

  # Every package a module declares, as a definition to be called with the
  # package set it is being built for. They go through an overlay so that
  # `pkgs.<name>` exists, which is what lets one package depend on another
  # and a devshell take `inputsFrom = [pkgs.<name>]`.
  pkgDefsFor = pkgs: let
    declared =
      if cfg.packages == null
      then {}
      else if isFunction cfg.packages
      then cfg.packages (moduleArgsOf pkgs)
      else cfg.packages;
  in
    declared
    // optionalAttrs (cfg.package != null) {default = cfg.package;};

  moduleArgsOf = pkgs: evaluated.config._module.args.moduleArgs // {inherit (pkgs) system;};

  # A definition may be written either way, and both are common here:
  #
  #   { lib, rustPlatform, ... }: ...   callPackage style, args from pkgs
  #   pkgs: ...                         the whole set, for a one-liner
  #
  # The second has no named arguments, so callPackage would hand it nothing;
  # it is wrapped to take `pkgs` explicitly. A definition naming itself gets
  # the previous package set's entry, so `foo = { foo, ... }: ...` overrides
  # rather than recurses.
  genPkg = final: prev: name: def: let
    args = builtins.functionArgs def;
    noArgs = args == {};
    def' =
      if noArgs
      then {pkgs}: def pkgs
      else def;
    dependsOnSelf = args ? ${name};
    dependsOnPkgs = noArgs || (args ? pkgs);
    selfOverride = {
      ${name} =
        prev.${name}
        or (throw "${name} depends on ${name}, but no existing ${name}.");
    };
    overrides =
      optionalAttrs dependsOnSelf selfOverride
      // optionalAttrs dependsOnPkgs {pkgs = final.pkgs // selfOverride;};
  in
    final.callPackage def' overrides;

  packageOverlay = final: prev: let
    defs = pkgDefsFor prev;
    called = mapAttrs (name: genPkg final prev name) defs;
  in
    called
    // optionalAttrs (cfg.package != null && cfg.pname != null) {
      ${cfg.pname} = called.default;
    };

  # What a package definition may ask for besides nixpkgs itself. A package
  # written here can take `inputs` or `system` the way it takes `lib`,
  # because callPackage looks them up in the package set and this is what
  # puts them there.
  argsOverlay = _final: prev: {
    inherit (prev.stdenv.hostPlatform) system;
    inherit src;
    inherit (cfg) inputs;
  };

  pkgsFor =
    genAttrs systems
    (system:
      import nixpkgs {
        inherit system;
        inherit (cfg.nixpkgs) config;
        overlays = cfg.withOverlays ++ [packageOverlay];
      });

  genSystems = f: genAttrs systems (system: f pkgsFor.${system});

  # A shell from its description: everything that is a function of pkgs is
  # applied, and anything in `env` becomes an environment variable.
  genDevShell = pkgs: shell:
    if shell.overrideShell != null
    then shell.overrideShell
    else let
      applied = mapAttrs (_: v:
        if isFunction v
        then v pkgs
        else v)
      (removeAttrs shell ["hardeningDisable" "overrideShell"]);
    in
      pkgs.mkShell.override {inherit (applied) stdenv;}
      (applied.env
        // {
          inherit (applied) inputsFrom packages shellHook;
          inherit (shell) hardeningDisable;
        });

  # One script that walks what it is given and dispatches per file. Written
  # rather than inherited so that `nix fmt` and the formatting check run the
  # same thing the devshell's hooks do.
  formattersScript = pkgs: let
    formatters =
      if isFunction cfg.formatters
      then cfg.formatters pkgs
      else cfg.formatters;
    # A `case` arm per pattern. Newline-separated rather than written across
    # lines, so the string carries no whitespace-only line for a formatter to
    # keep rewriting.
    arms =
      lib.concatMapStrings (
        pattern: "\n  ${pattern}) ${formatters.${pattern}} \"$f\" & ;;"
      )
      (builtins.attrNames formatters);
  in
    pkgs.writeShellScriptBin "formatter" ''
      if [ $# -eq 0 ]; then
        flakedir=.
        while [ "$(${pkgs.coreutils}/bin/realpath "$flakedir")" != / ]; do
          if [ -e "$flakedir/flake.nix" ]; then
            exec "$0" "$flakedir"
          fi
          flakedir="$flakedir/.."
        done
        echo "Failed to find flake root" >&2
        exit 1
      fi
      for f in "$@"; do
        if [ -d "$f" ]; then
          ${pkgs.fd}/bin/fd "$f" -Htf -x "$0" &
        else
          case "$(${pkgs.coreutils}/bin/basename "$f")" in${arms}
          esac
        fi
      done &>/dev/null
      wait
    '';

  formatterFor = pkgs:
    if cfg.formatter != null
    then cfg.formatter pkgs
    else formattersScript pkgs;

  hasFormatter = cfg.formatter != null || cfg.formatters != null;

  # Formatting is a check so that `nix flake check` fails on an unformatted
  # tree, which is the whole reason a generated flake is formatted before it
  # is written.
  formattingCheck = pkgs:
    pkgs.runCommand "check-formatting" {} ''
      cp -r --no-preserve=mode ${src} ./src
      cd ./src
      ${lib.getExe (formatterFor pkgs)} . >/dev/null
      if ! ${pkgs.diffutils}/bin/diff -qr ${src} . >/tmp/formatting-diff; then
        echo "not formatted:" >&2
        sed 's/Files .* and \(.*\) differ/  \1/' /tmp/formatting-diff >&2
        exit 1
      fi
      touch $out
    '';

  # `inputs'.foo.packages` is `inputs.foo.packages.<system>`: the same set
  # with this system already chosen.
  selectAttr = system: mapAttrs (_: v: v.${system} or null);

  # Already a built system, rather than a specification to build. Checked
  # by looking for the attribute rather than by testing whether it is a
  # derivation, because the latter evaluates the whole NixOS module set.
  isBuiltSystem = x: x ? config.system.build.toplevel;

  coreModule = {
    config,
    moduleArgs,
    ...
  }: let
    mkNixos = hostname: spec:
      nixpkgs.lib.nixosSystem (spec
        // {
          specialArgs =
            {
              inherit hostname;
              inherit (config) inputs;
            }
            // spec.specialArgs or {};
          modules =
            [
              config.propagationModule
              ({flake, ...}: {_module.args = {inherit (flake) inputs';};})
            ]
            ++ spec.modules or [];
        });

    nixosSystems =
      mapAttrs
      (hostname: spec:
        if isBuiltSystem spec
        then spec
        else mkNixos hostname spec)
      config.nixosConfigurations;
  in {
    options = {
      inputs = mkOption {
        type = types.lazyAttrsOf types.raw;
        default = {};
      };

      systems = mkOption {
        type = types.listOf types.str;
        default = ["x86_64-linux" "aarch64-linux"];
      };

      description = mkOption {
        type = types.nullOr types.str;
        default = null;
      };

      # Everything ends up here, and this is what the flake is.
      outputs = mkOption {
        type = projectTypes.optCallWith moduleArgs (types.lazyAttrsOf outputsType);
        default = {};
      };

      # A single package, as a callPackage-style definition. It becomes
      # packages.default, packages.<pname>, an overlay entry under both
      # names, and a check.
      package = mkOption {
        type = types.nullOr projectTypes.packageDef;
        default = null;
      };

      pname = mkOption {
        type = types.nullOr types.str;
        default = null;
      };

      # The overlay the declared packages are resolved through, exposed
      # because a module that filters packages by platform needs to ask what
      # they are without building them.
      packageOverlay = mkOption {
        type = projectTypes.overlay;
        default = packageOverlay;
      };

      packages = mkOption {
        type = types.nullOr (projectTypes.optFunctionTo (types.lazyAttrsOf types.raw));
        default = null;
      };

      checks = mkOption {
        type = types.nullOr (projectTypes.optFunctionTo (types.lazyAttrsOf types.raw));
        default = null;
      };

      devShell = mkOption {
        type = types.nullOr devShellType;
        default = null;
      };

      # Keyed by name, each entry a function of pkgs returning the shell's
      # attributes. Not a function over the whole set: a module that adds one
      # shell should not have to know about the others, and one that renames
      # them all needs to see them as a set.
      devShells = mkOption {
        type = projectTypes.optCallWith moduleArgs (types.lazyAttrsOf devShellType);
        default = {};
      };

      formatter = mkOption {
        type = types.nullOr (types.functionTo types.package);
        default = null;
      };

      # A command per file pattern, dispatched by one script. The patterns
      # are shell `case` patterns matched against a file's basename, and the
      # command is invoked with the path as its last argument.
      #
      # No defaults are supplied: a tree that wants nix files formatted says
      # so. Inheriting a set of them is how a formatter ends up running on
      # something nobody chose - and this tree already had to override two
      # inherited ones with no-ops to stop them fighting a generator.
      formatters = mkOption {
        type = types.nullOr (projectTypes.optFunctionTo (types.lazyAttrsOf types.str));
        default = null;
      };

      overlays = mkOption {
        type = types.lazyAttrsOf projectTypes.overlay;
        default = {};
      };

      withOverlays = mkOption {
        type = types.listOf projectTypes.overlay;
        default = [];
      };

      nixpkgs.config = mkOption {
        type = types.attrsOf types.raw;
        default = {};
      };

      lib = mkOption {
        type = types.lazyAttrsOf types.raw;
        default = {};
      };

      nixosModules = mkOption {
        type = projectTypes.optCallWith moduleArgs (types.lazyAttrsOf projectTypes.module);
        default = {};
      };

      # A specification - `{ system, modules }` - or a system already built.
      # A specification is built here, with the propagation module in front
      # of it so the result shares this flake's overlays and can reach its
      # inputs.
      nixosConfigurations = mkOption {
        type = projectTypes.optCallWith moduleArgs (types.lazyAttrsOf (projectTypes.optCallWith moduleArgs types.attrs));
        default = {};
      };

      homeModules = mkOption {
        type = projectTypes.optCallWith moduleArgs (types.lazyAttrsOf projectTypes.module);
        default = {};
      };

      templates = mkOption {
        type = types.lazyAttrsOf types.raw;
        default = {};
      };

      apps = mkOption {
        type = types.nullOr (projectTypes.optFunctionTo (types.lazyAttrsOf types.raw));
        default = null;
      };

      legacyPackages = mkOption {
        type = types.nullOr (projectTypes.optFunctionTo (types.lazyAttrsOf types.raw));
        default = null;
      };

      # A module to add to a module system nested inside this one - a NixOS
      # or home-manager configuration - so it inherits the overlays and the
      # nixpkgs config this flake was built with, and can reach the flake's
      # arguments under `flake`. Without it a nested configuration silently
      # builds against a different package set than everything else here.
      propagationModule = mkOption {
        type = projectTypes.module;
        internal = true;
      };

      # Makes the resulting flake callable, which is how a consuming repo
      # says `inputs.project ./. { ... }` instead of importing a module.
      functor = mkOption {
        type = types.nullOr (types.functionTo types.raw);
        default = null;
      };
    };

    config.propagationModule = let
      flakeConfig = config;
    in
      {
        lib,
        pkgs,
        options,
        config,
        ...
      }: let
        inherit (pkgs.stdenv.hostPlatform) system;

        # The flake's own arguments, reachable from inside the nested system
        # under `flake`, with the per-system choices already made.
        propArgs._module.args.flake =
          {
            inputs' = selectAttr system flakeConfig.inputs;
            outputs' = selectAttr system flakeConfig.outputs;
          }
          // moduleArgs;
      in {
        config =
          lib.optionalAttrs (options ? nixpkgs) {
            nixpkgs =
              lib.optionalAttrs (options ? nixpkgs.overlays) {
                overlays = lib.mkOrder 10 (flakeConfig.withOverlays ++ [flakeConfig.packageOverlay]);
              }
              // lib.optionalAttrs (options ? nixpkgs.config) {
                inherit (flakeConfig.nixpkgs) config;
              };
          }
          // lib.optionalAttrs (options ? home-manager.sharedModules) {
            home-manager.sharedModules =
              if config.home-manager.useGlobalPkgs
              then [propArgs]
              else [flakeConfig.propagationModule];
          }
          // propArgs;
      };

    # Ordered early, and a definition of withOverlays rather than a list
    # prepended at the end, so that everything which forwards the overlays -
    # a NixOS or home-manager configuration built through propagationModule -
    # carries it too. A nested system that lost it would build against a
    # package set where `pkgs.inputs` did not exist.
    config.withOverlays = lib.mkOrder 10 [argsOverlay];

    # What every module can take as an argument, alongside the ones the
    # module system provides itself. `inputs` is the one modules here reach
    # for most: a NixOS configuration is written as a function of it.
    config._module.args = {
      inherit (config) inputs;
      inherit pkgsFor genSystems src;
    };

    # Each named option writes into `outputs`, so a module that needs
    # something with no option can write there directly and be merged the
    # same way.
    config.outputs = let
      perSystem = name: opt:
        optionalAttrs (opt != null) {
          ${name} = genSystems (pkgs:
            if isFunction opt
            then opt pkgs
            else opt);
        };

      # Read back out of pkgs rather than from the definitions, so a package
      # sees the same set every other package sees.
      packageNames = pkgs: builtins.attrNames (packageOverlay pkgs pkgs);
      packagesOut = genSystems (pkgs: genAttrs (packageNames pkgs) (n: pkgs.${n}));
      anyPackages = config.package != null || config.packages != null;
    in
      foldl recursiveUpdate {} [
        (optionalAttrs anyPackages {
          packages = packagesOut;
          checks = mapAttrs (_: mapAttrs' (n: nameValuePair "packages-${n}")) packagesOut;
          overlays.default = _final: prev: removeAttrs (packageOverlay prev prev) ["default"];
        })
        (perSystem "checks" config.checks)
        (optionalAttrs (config.devShells != {}) {
          devShells = genSystems (pkgs: mapAttrs (_: v: genDevShell pkgs (v pkgs)) config.devShells);
        })
        (perSystem "apps" config.apps)
        (perSystem "legacyPackages" config.legacyPackages)
        (optionalAttrs (config.devShell != null) {
          devShells = genSystems (pkgs: {default = genDevShell pkgs (config.devShell pkgs);});
        })
        (optionalAttrs hasFormatter {
          formatter = genSystems formatterFor;
          checks = genSystems (pkgs: {formatting = formattingCheck pkgs;});
        })

        # One check per NixOS configuration, so `nix flake check` builds the
        # systems. The derivation is wrapped rather than used directly
        # because computing its name is expensive enough to slow `nix flake
        # show` to a crawl.
        (optionalAttrs (config.nixosConfigurations != {}) {
          nixosConfigurations = nixosSystems;
          checks = lib.foldl lib.recursiveUpdate {} (lib.mapAttrsToList (n: v: {
              ${v.pkgs.stdenv.buildPlatform.system}."nixos-${n}" =
                pkgsFor.${v.pkgs.stdenv.buildPlatform.system}.runCommand "check-nixos-${n}" {}
                "echo ${v.config.system.build.toplevel} > $out";
            })
            nixosSystems);
        })
        (optionalAttrs (config.overlays != {}) {inherit (config) overlays;})
        (optionalAttrs (config.lib != {}) {inherit (config) lib;})
        (optionalAttrs (config.nixosModules != {}) {inherit (config) nixosModules;})
        (optionalAttrs (config.homeModules != {}) {inherit (config) homeModules;})
        (optionalAttrs (config.templates != {}) {inherit (config) templates;})
        (optionalAttrs (config.description != null) {inherit (config) description;})
      ];
  };
in
  cfg.outputs
  // optionalAttrs (cfg.functor != null) {__functor = cfg.functor;}
