# The nix-workspace entry point: `mkWorkspace`, called from a flake.nix as
#
#   outputs = inputs:
#     inputs.nix-workspace ./. {
#       inherit inputs;
#     };
#
# and running the pipeline the phase headers below mark out: discovery, then
# Nickel evaluation through IFD, then namespacing with conflict (NW2xx) and
# dependency (NW3xx) detection, then the flake outputs themselves.
{
  nixpkgs,
  nix-workspace,
}: let
  inherit (nixpkgs) lib;

  discover = import ./discover.nix {inherit lib;};
  evalNickel = import ./eval-nickel.nix {inherit lib;};
  systemsLib = import ./systems.nix {inherit lib;};
  namespacingLib = import ./namespacing.nix {inherit lib;};
  pluginsLib = import ./plugins.nix {inherit lib;};
  packageBuilder = import ./builders/packages.nix {inherit lib;};
  shellBuilder = import ./builders/shells.nix {inherit lib;};
  machineBuilder = import ./builders/machines.nix {inherit lib;};
  moduleBuilder = import ./builders/modules.nix {inherit lib;};
  overlayBuilder = import ./builders/overlays.nix {inherit lib;};
  checkBuilder = import ./builders/checks.nix {inherit lib;};
  templateBuilder = import ./builders/templates.nix {inherit lib;};

  # The contracts and plugins directories shipped with nix-workspace.
  contractsDir = nix-workspace.contracts;
  pluginsDir = nix-workspace.plugins;

  # ── mkWorkspace ─────────────────────────────────────────────────
  mkWorkspace = workspaceRoot: config: let
    inputs = config.inputs or {};
    userNixpkgs = inputs.nixpkgs or nixpkgs;

    # ── Phase 1: Discovery ──────────────────────────────────────
    discovered = discover.discoverAll workspaceRoot (config.conventions or null);

    discoveredPackages = discovered.packages or {};
    discoveredShells = discovered.shells or {};
    discoveredMachines = discovered.machines or {};
    discoveredModules = discovered.modules or {};
    discoveredHome = discovered.home or {};
    discoveredOverlays = discovered.overlays or {};
    discoveredChecks = discovered.checks or {};
    discoveredTemplates = discovered.templates or {};

    hasNclFiles =
      (discoveredPackages != {})
      || (discoveredShells != {})
      || (discoveredMachines != {})
      || (discoveredModules != {})
      || (discoveredHome != {})
      || (discoveredOverlays != {})
      || (discoveredChecks != {})
      || (discoveredTemplates != {})
      || lib.pathExists (workspaceRoot + "/workspace.ncl");

    # ── Phase 1c: Plugin resolution ─────────────────────────────
    #
    # Which plugins are wanted has to be known BEFORE the full Nickel
    # evaluation, because their .ncl paths go into the wrapper generator. So
    # plugins may be declared Nix-side as well as in workspace.ncl, and the
    # Nix-side `config.plugins` wins where both do.
    requestedPlugins = config.plugins or [];

    pluginNclPaths =
      map (name: pluginsLib.resolvePluginNcl pluginsDir name)
      requestedPlugins;

    loadedPlugins =
      if requestedPlugins != []
      then pluginsLib.loadPlugins pluginsDir requestedPlugins
      else {
        builders = {};
        shellExtras = {};
        pluginNames = [];
      };

    pluginValidation = pluginsLib.validatePlugins requestedPlugins;
    pluginsValid =
      if pluginValidation != []
      then let
        formatDiag = d:
          "[${d.code}] ${d.message}"
          + (
            if d ? hint
            then "\n  hint: ${d.hint}"
            else ""
          );
        msg = lib.concatStringsSep "\n\n" (map formatDiag pluginValidation);
      in
        throw "nix-workspace: plugin errors:\n\n${msg}"
      else true;

    # A module is two files: .ncl for the Nickel config, .nix for the NixOS
    # implementation.
    discoveredModuleNixFiles = moduleBuilder.discoverNixFiles workspaceRoot "modules";
    discoveredHomeNixFiles = moduleBuilder.discoverNixFiles workspaceRoot "home";

    # ── Phase 1b: Subworkspace discovery ────────────────────────
    #
    # A subdirectory holding a workspace.ncl. VCS-agnostic: git submodules, jj
    # checkouts, plain directories and symlinks all behave the same.
    subworkspaceMap = discover.discoverAllSubworkspaces workspaceRoot;
    hasSubworkspaces = subworkspaceMap != {};

    # ── Phase 2: Nickel evaluation ──────────────────────────────
    #
    # A bootstrap pkgs, only to get the `nickel` binary the IFD needs.
    # builtins.currentSystem where it is available, x86_64-linux otherwise.
    # Bootstrap fallback for impure callers; flake outputs pass the system explicitly.
    # ast-grep-ignore: nix-currentsystem
    bootstrapSystem = builtins.currentSystem or "x86_64-linux";
    bootstrapPkgs = import userNixpkgs {system = bootstrapSystem;};

    workspaceConfig = assert pluginsValid;
      if hasNclFiles
      then
        evalNickel.evalWorkspace {
          inherit
            bootstrapPkgs
            contractsDir
            workspaceRoot
            discoveredPackages
            discoveredShells
            discoveredMachines
            discoveredModules
            discoveredHome
            discoveredOverlays
            discoveredChecks
            discoveredTemplates
            pluginNclPaths
            ;
        }
      else evalNickel.emptyConfig;

    # ── Phase 2c: Plugin config evaluation ──────────────────────
    evaluatedPluginConfigs =
      if requestedPlugins != []
      then
        evalNickel.evalAllPlugins {
          inherit bootstrapPkgs contractsDir;
          pluginNclPaths = lib.listToAttrs (
            map (name: {
              inherit name;
              value = pluginsLib.resolvePluginNcl pluginsDir name;
            })
            requestedPlugins
          );
        }
      else {};

    pluginConventions =
      if evaluatedPluginConfigs != {}
      then pluginsLib.extractConventions evaluatedPluginConfigs
      else {};

    pluginDiscovered =
      if pluginConventions != {}
      then
        pluginsLib.discoverPluginConventions
        discover.discoverNclFiles
        workspaceRoot
        pluginConventions
      else {};

    # ── Phase 2b: Subworkspace Nickel evaluation ────────────────
    #
    # One pass each, producing independent validated config trees.
    subworkspaceConfigs =
      if hasSubworkspaces
      then
        evalNickel.evalAllSubworkspaces {
          inherit bootstrapPkgs contractsDir subworkspaceMap pluginNclPaths;
        }
      else {};

    # ── Phase 3: Namespacing and conflict detection ─────────────

    subworkspaceEntries =
      lib.mapAttrsToList (
        name: subConfig: {
          inherit name;
          outputs = {
            packages = subConfig.packages or {};
            shells = subConfig.shells or {};
            machines = subConfig.machines or {};
            modules = subConfig.modules or {};
            home = subConfig.home or {};
            overlays = subConfig.overlays or {};
            checks = subConfig.checks or {};
            templates = subConfig.templates or {};
          };
        }
      )
      subworkspaceConfigs;

    rootOutputsForConflictCheck = {
      packages = workspaceConfig.packages or {};
      shells = workspaceConfig.shells or {};
      machines = workspaceConfig.machines or {};
      modules = workspaceConfig.modules or {};
      home = workspaceConfig.home or {};
      overlays = workspaceConfig.overlays or {};
      checks = workspaceConfig.checks or {};
      templates = workspaceConfig.templates or {};
    };

    mergedOutputs =
      if hasSubworkspaces
      then namespacingLib.mergeOutputs rootOutputsForConflictCheck subworkspaceEntries
      else rootOutputsForConflictCheck;

    # ── Phase 3b: Dependency validation ─────────────────────────
    #
    # Throws on a dependency naming a subworkspace that does not exist, and on
    # a cycle.
    dependenciesValid =
      if hasSubworkspaces
      then let
        subConfigsForValidation =
          lib.mapAttrs (
            _name: subConfig: {
              dependencies = subConfig.dependencies or {};
            }
          )
          subworkspaceConfigs;
        diagnostics = namespacingLib.validateAllDependencies subConfigsForValidation;
      in
        if diagnostics != []
        then let
          formatDiag = d:
            "[${d.code}] ${d.message}"
            + (
              if d ? hint
              then "\n  hint: ${d.hint}"
              else ""
            );
          msg = lib.concatStringsSep "\n\n" (map formatDiag diagnostics);
        in
          throw "nix-workspace: dependency errors:\n\n${msg}"
        else true
      else true;

    # The Nix-side config may override fields from workspace.ncl, and items
    # found under a plugin convention directory (crates/, say) join the packages
    # config carrying their plugin builder defaults.
    pluginPackageConfigs = let
      pkgConventions =
        lib.filterAttrs (
          _name: conv: conv.output == "packages"
        )
        pluginConventions;

      allPluginPkgs =
        lib.foldl' (
          acc: convName: let
            items = pluginDiscovered.${convName} or {};
          in
            acc
            // (lib.mapAttrs (
                _name: item: {
                  build-system = item.builder;
                }
              )
              items)
        ) {}
        (lib.attrNames pkgConventions);
    in
      allPluginPkgs;

    effectiveConfig = assert dependenciesValid;
      workspaceConfig
      // (lib.optionalAttrs (config ? systems) {inherit (config) systems;})
      // (lib.optionalAttrs (config ? nixpkgs) {
        nixpkgs = (workspaceConfig.nixpkgs or {}) // config.nixpkgs;
      });

    systems = effectiveConfig.systems or systemsLib.defaultSystems;
    nixpkgsConfig = let
      ncl = effectiveConfig.nixpkgs or {};
    in
      (lib.optionalAttrs (ncl ? allow-unfree) {allowUnfree = ncl.allow-unfree;})
      // (ncl.config or {});

    packageConfigs = (mergedOutputs.packages or {}) // pluginPackageConfigs;
    shellConfigs = mergedOutputs.shells or {};
    machineConfigs = mergedOutputs.machines or {};
    moduleConfigs = mergedOutputs.modules or {};
    homeConfigs = mergedOutputs.home or {};
    overlayConfigs = mergedOutputs.overlays or {};
    checkConfigs = mergedOutputs.checks or {};
    templateConfigs = mergedOutputs.templates or {};

    # ── Phase 4: Build outputs ──────────────────────────────────

    # ── Per-system outputs (packages, devShells) ────────────────
    perSystemOutputs = systemsLib.eachSystem systems (
      system: let
        pkgs = import userNixpkgs {
          inherit system;
          config = nixpkgsConfig;
        };

        coreBuilders = {
          generic = packageBuilder.buildGeneric;
          rust = packageBuilder.buildRust;
          go = packageBuilder.buildGo;
        };

        builtPackages =
          lib.mapAttrs (
            name: cfg: let
              effectiveCfg = pluginsLib.applyBuilderDefaults loadedPlugins.builders cfg;

              # A package from a subworkspace resolves against that root.
              effectiveRoot = resolvePackageRoot name;
            in
              pluginsLib.routeBuilder
              loadedPlugins.builders
              coreBuilders
              pkgs
              effectiveRoot
              name
              effectiveCfg
          )
          (
            lib.filterAttrs (
              _: cfg:
                lib.elem system (cfg.systems or systems)
            )
            packageConfigs
          );

        pluginShellExtras = pluginsLib.collectShellExtras loadedPlugins.shellExtras pkgs;

        builtShells =
          lib.mapAttrs (
            name: cfg:
              shellBuilder.buildShell pkgs name cfg builtPackages pluginShellExtras
          )
          (
            lib.filterAttrs (
              _: cfg:
                lib.elem system (cfg.systems or systems)
            )
            shellConfigs
          );

        # If there's exactly one package and no explicit default shell,
        # create a default shell with that package's build inputs.
        hasDefaultShell = builtShells ? default;
        packageNames = lib.attrNames builtPackages;
        autoDefaultShell =
          if !hasDefaultShell && lib.length packageNames == 1
          then let
            singlePkgName = lib.head packageNames;
          in {
            default = pkgs.mkShell {
              name = "nix-workspace-default";
              inputsFrom = [builtPackages.${singlePkgName}];
            };
          }
          else {};
      in
        (lib.optionalAttrs (builtPackages != {}) {
          packages.${system} = builtPackages;
        })
        // (lib.optionalAttrs (builtShells != {} || autoDefaultShell != {}) {
          devShells.${system} = builtShells // autoDefaultShell;
        })
        // (lib.optionalAttrs (checkConfigs != {}) {
          checks.${system} = checkBuilder.buildAllChecks {
            inherit pkgs workspaceRoot system;
            workspaceSystems = systems;
            inherit checkConfigs;
            workspacePackages = builtPackages;
            discoveredPaths = {};
          };
        })
    );

    # ── Resolve workspace root for namespaced outputs ───────────
    #
    # When a package/module came from a subworkspace, we need to use
    # that subworkspace's root for source resolution, not the root
    # workspace's root.

    # Build a mapping: namespacedOutputName → subworkspaceRoot
    # for all subworkspace outputs across all conventions
    subworkspaceOutputRoots = let
      subEntries = lib.concatLists (
        lib.mapAttrsToList (
          subName: subConfig: let
            subPkgs = subConfig.packages or {};
            namespacedNames =
              lib.mapAttrsToList (
                outputName: _:
                  namespacingLib.namespacedName subName outputName
              )
              subPkgs;
          in
            map (nsName: {
              name = nsName;
              value = subworkspaceMap.${subName}.path;
            })
            namespacedNames
        )
        subworkspaceConfigs
      );
    in
      lib.listToAttrs subEntries;

    resolvePackageRoot = pkgName:
      subworkspaceOutputRoots.${pkgName} or workspaceRoot;

    # ── Non-per-system outputs (nixosConfigurations, modules) ───
    #
    # The flake schema has these outside the per-system tree: a NixOS
    # configuration declares its own system internally.

    # The discovered .nix files, plus any paths the Nickel module configs name.
    resolvedModulePaths = let
      nixPaths = discoveredModuleNixFiles;
      nclPaths =
        lib.filterAttrs (_: cfg: cfg ? path) (workspaceConfig.modules or {});
      nclResolvedPaths =
        lib.mapAttrs (
          _: cfg:
            if lib.hasPrefix "./" cfg.path || lib.hasPrefix "../" cfg.path
            then workspaceRoot + "/${cfg.path}"
            else if lib.hasPrefix "/" cfg.path
            then /. + cfg.path
            else workspaceRoot + "/${cfg.path}"
        )
        nclPaths;
    in
      nixPaths // nclResolvedPaths;

    resolvedHomePaths = let
      nixPaths = discoveredHomeNixFiles;
      nclPaths =
        lib.filterAttrs (_: cfg: cfg ? path) (workspaceConfig.home or {});
      nclResolvedPaths =
        lib.mapAttrs (
          _: cfg:
            if lib.hasPrefix "./" cfg.path || lib.hasPrefix "../" cfg.path
            then workspaceRoot + "/${cfg.path}"
            else if lib.hasPrefix "/" cfg.path
            then /. + cfg.path
            else workspaceRoot + "/${cfg.path}"
        )
        nclPaths;
    in
      nixPaths // nclResolvedPaths;

    subworkspaceModulePaths = let
      allSubModulePaths = lib.concatLists (
        lib.mapAttrsToList (
          subName: subInfo: let
            subRoot = subInfo.path;
            subNixFiles = moduleBuilder.discoverNixFiles subRoot "modules";
            namespacedNixFiles =
              lib.mapAttrs' (
                name: path: {
                  name = namespacingLib.namespacedName subName name;
                  value = path;
                }
              )
              subNixFiles;
          in
            lib.mapAttrsToList (name: value: {inherit name value;}) namespacedNixFiles
        )
        subworkspaceMap
      );
    in
      lib.listToAttrs allSubModulePaths;

    subworkspaceHomePaths = let
      allSubHomePaths = lib.concatLists (
        lib.mapAttrsToList (
          subName: subInfo: let
            subRoot = subInfo.path;
            subNixFiles = moduleBuilder.discoverNixFiles subRoot "home";
            namespacedNixFiles =
              lib.mapAttrs' (
                name: path: {
                  name = namespacingLib.namespacedName subName name;
                  value = path;
                }
              )
              subNixFiles;
          in
            lib.mapAttrsToList (name: value: {inherit name value;}) namespacedNixFiles
        )
        subworkspaceMap
      );
    in
      lib.listToAttrs allSubHomePaths;

    allModulePaths = resolvedModulePaths // subworkspaceModulePaths;
    allHomePaths = resolvedHomePaths // subworkspaceHomePaths;

    nixosConfigurations =
      if machineConfigs != {}
      then
        machineBuilder.buildAllMachines {
          nixpkgs = userNixpkgs;
          inherit workspaceRoot machineConfigs;
          workspaceModules = allModulePaths;
          homeModules = allHomePaths;
          extraInputs = inputs;
        }
      else {};

    nixosModules =
      if moduleConfigs != {} || allModulePaths != {}
      then let
        effectiveModuleConfigs =
          (lib.mapAttrs (_: _: {}) allModulePaths)
          // moduleConfigs;
      in
        moduleBuilder.buildAllNixosModules {
          inherit workspaceRoot;
          moduleConfigs = effectiveModuleConfigs;
          discoveredPaths = allModulePaths;
        }
      else {};

    homeModules =
      if homeConfigs != {} || allHomePaths != {}
      then let
        effectiveHomeConfigs =
          (lib.mapAttrs (_: _: {}) allHomePaths)
          // homeConfigs;
      in
        moduleBuilder.buildAllHomeModules {
          inherit workspaceRoot;
          homeConfigs = effectiveHomeConfigs;
          discoveredPaths = allHomePaths;
        }
      else {};

    overlays =
      if overlayConfigs != {}
      then
        overlayBuilder.buildAllOverlays {
          inherit workspaceRoot overlayConfigs;
        }
      else {};

    templates =
      if templateConfigs != {}
      then
        templateBuilder.buildAllTemplates {
          inherit workspaceRoot templateConfigs;
        }
      else {};
  in
    perSystemOutputs
    // (lib.optionalAttrs (nixosConfigurations != {}) {
      inherit nixosConfigurations;
    })
    // (lib.optionalAttrs (nixosModules != {}) {
      inherit nixosModules;
    })
    // (lib.optionalAttrs (homeModules != {}) {
      inherit homeModules;
    })
    // (lib.optionalAttrs (overlays != {}) {
      inherit overlays;
    })
    // (lib.optionalAttrs (templates != {}) {
      inherit templates;
    })
    // (lib.optionalAttrs (requestedPlugins != []) {
      _pluginMeta =
        lib.mapAttrs (
          _name: cfg: {
            name = cfg.name or "unknown";
            description = cfg.description or "";
            conventions = lib.attrNames (cfg.conventions or {});
          }
        )
        evaluatedPluginConfigs;
    });
in {
  inherit mkWorkspace;

  inherit discover systemsLib namespacingLib packageBuilder shellBuilder machineBuilder moduleBuilder evalNickel;
}
