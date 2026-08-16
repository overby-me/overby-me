# nix-workspace library — main entry point
#
# This module exports the `mkWorkspace` function that serves as the primary
# interface for nix-workspace. It is designed to be called from a flake.nix:
#
#   outputs = inputs:
#     inputs.nix-workspace ./. {
#       inherit inputs;
#     };
#
# The function orchestrates the full pipeline:
#   1. Discovery  — scan convention directories for .ncl files
#   2. Subworkspace discovery — find subdirectories with workspace.ncl
#   3. Evaluation — run Nickel to validate and export configs as JSON (IFD)
#   4. Namespacing — prefix subworkspace outputs with directory names
#   5. Conflict detection — check for naming collisions (NW2xx diagnostics)
#   6. Dependency validation — check cross-subworkspace references (NW3xx)
#   7. Building   — construct flake outputs from the validated config
#
# Updated for v0.4 to support plugins with contract extensions,
# custom convention directories, and plugin-provided builders.
#
{
  nixpkgs,
  nix-workspace,
}: let
  inherit (nixpkgs) lib;

  # Import sub-modules, passing the shared `lib`.
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
  #
  # Type: Path -> AttrSet -> AttrSet
  #
  # Arguments:
  #   workspaceRoot — Path to the workspace directory (typically ./.)
  #   config        — Configuration attrset, must contain at least `inputs`
  #     config.inputs       — The flake inputs (must include `nixpkgs`)
  #     config.systems      — Optional list of systems (overrides workspace.ncl)
  #     config.nixpkgs      — Optional nixpkgs config overrides
  #
  # Returns: A standard flake outputs attrset.
  #
  mkWorkspace = workspaceRoot: config: let
    inputs = config.inputs or {};
    userNixpkgs = inputs.nixpkgs or nixpkgs;

    # ── Phase 1: Discovery ──────────────────────────────────────
    #
    # Scan the workspace root for convention directories and .ncl files.
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
      || builtins.pathExists (workspaceRoot + "/workspace.ncl");

    # ── Phase 1c: Plugin resolution ─────────────────────────────
    #
    # Peek at workspace.ncl to determine which plugins are requested.
    # We need the plugin list BEFORE full Nickel evaluation because
    # plugin .ncl paths must be passed to the wrapper generator.
    #
    # Strategy: do a lightweight Nickel eval of just the plugins field,
    # or read it from the config. For simplicity, we support plugins
    # declared in the Nix-side config as well as in workspace.ncl.
    #
    # The Nix-side `config.plugins` takes precedence if provided.
    # Otherwise, we'll pass plugin paths to the Nickel evaluator and
    # let it handle validation.
    requestedPlugins = config.plugins or [];

    # Resolve plugin names to their .ncl definition file paths
    pluginNclPaths =
      map (name: pluginsLib.resolvePluginNcl pluginsDir name)
      requestedPlugins;

    # Load plugin Nix-side builders and shell extras
    loadedPlugins =
      if requestedPlugins != []
      then pluginsLib.loadPlugins pluginsDir requestedPlugins
      else {
        builders = {};
        shellExtras = {};
        pluginNames = [];
      };

    # Validate plugins (check for duplicates, etc.)
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
        msg = builtins.concatStringsSep "\n\n" (map formatDiag pluginValidation);
      in
        throw "nix-workspace: plugin errors:\n\n${msg}"
      else true;

    # Discover .nix implementation files alongside .ncl configs for modules.
    # Modules have a dual structure: .ncl for Nickel config, .nix for NixOS implementation.
    discoveredModuleNixFiles = moduleBuilder.discoverNixFiles workspaceRoot "modules";
    discoveredHomeNixFiles = moduleBuilder.discoverNixFiles workspaceRoot "home";

    # ── Phase 1b: Subworkspace discovery ────────────────────────
    #
    # Scan for subdirectories containing workspace.ncl files.
    # This is VCS-agnostic: git submodules, jj checkouts, plain dirs,
    # and symlinks all work identically.
    subworkspaceMap = discover.discoverAllSubworkspaces workspaceRoot;
    hasSubworkspaces = subworkspaceMap != {};

    # ── Phase 2: Nickel evaluation ──────────────────────────────
    #
    # Evaluate workspace.ncl (and discovered .ncl files) through Nickel
    # using IFD. This produces a fully validated JSON configuration tree.
    #
    # We need a "bootstrap" pkgs to obtain the `nickel` binary for IFD.
    # Use builtins.currentSystem when available, otherwise default to x86_64-linux.
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
    #
    # Evaluate each plugin's plugin.ncl through Nickel to extract
    # convention mappings and builder metadata as JSON.
    evaluatedPluginConfigs =
      if requestedPlugins != []
      then
        evalNickel.evalAllPlugins {
          inherit bootstrapPkgs contractsDir;
          pluginNclPaths = builtins.listToAttrs (
            map (name: {
              inherit name;
              value = pluginsLib.resolvePluginNcl pluginsDir name;
            })
            requestedPlugins
          );
        }
      else {};

    # Extract plugin convention directory mappings
    pluginConventions =
      if evaluatedPluginConfigs != {}
      then pluginsLib.extractConventions evaluatedPluginConfigs
      else {};

    # Discover items from plugin convention directories
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
    # Each subworkspace gets its own Nickel evaluation pass.
    # This produces independent validated config trees.
    subworkspaceConfigs =
      if hasSubworkspaces
      then
        evalNickel.evalAllSubworkspaces {
          inherit bootstrapPkgs contractsDir subworkspaceMap pluginNclPaths;
        }
      else {};

    # ── Phase 3: Namespacing and conflict detection ─────────────
    #
    # Apply automatic namespacing to subworkspace outputs and check
    # for naming collisions.

    # Build the subworkspace entries for the namespacing module
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

    # Merge root + subworkspace outputs with namespacing and conflict detection
    mergedOutputs =
      if hasSubworkspaces
      then namespacingLib.mergeOutputs rootOutputsForConflictCheck subworkspaceEntries
      else rootOutputsForConflictCheck;

    # ── Phase 3b: Dependency validation ─────────────────────────
    #
    # Validate cross-subworkspace dependency declarations.
    # Throws if any dependency references a nonexistent subworkspace or
    # if there are circular dependencies.
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
          msg = builtins.concatStringsSep "\n\n" (map formatDiag diagnostics);
        in
          throw "nix-workspace: dependency errors:\n\n${msg}"
        else true
      else true;

    # Allow the Nix-side config to override certain fields from workspace.ncl.
    # Merge plugin-discovered items into the effective outputs.
    # Items from plugin convention directories (e.g. crates/) are added
    # to the packages config with their plugin builder defaults applied.
    pluginPackageConfigs = let
      # Flatten all plugin convention discoveries that map to "packages"
      pkgConventions =
        lib.filterAttrs (
          _name: conv: conv.output == "packages"
        )
        pluginConventions;

      # Collect all discovered items from package-targeting conventions
      allPluginPkgs =
        builtins.foldl' (
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
        (builtins.attrNames pkgConventions);
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

    # Use merged outputs (root + namespaced subworkspaces) for all config maps,
    # plus items discovered from plugin convention directories.
    packageConfigs = (mergedOutputs.packages or {}) // pluginPackageConfigs;
    shellConfigs = mergedOutputs.shells or {};
    machineConfigs = mergedOutputs.machines or {};
    moduleConfigs = mergedOutputs.modules or {};
    homeConfigs = mergedOutputs.home or {};
    overlayConfigs = mergedOutputs.overlays or {};
    checkConfigs = mergedOutputs.checks or {};
    templateConfigs = mergedOutputs.templates or {};

    # ── Phase 4: Build outputs ──────────────────────────────────
    #
    # Construct standard flake outputs with system multiplexing.

    # ── Per-system outputs (packages, devShells) ────────────────
    perSystemOutputs = systemsLib.eachSystem systems (
      system: let
        pkgs = import userNixpkgs {
          inherit system;
          config = nixpkgsConfig;
        };

        # Core builders — these are always available
        coreBuilders = {
          generic = packageBuilder.buildGeneric;
          rust = packageBuilder.buildRust;
          go = packageBuilder.buildGo;
        };

        # Build packages for this system, routing to plugin builders when available
        builtPackages =
          lib.mapAttrs (
            name: cfg: let
              # Apply plugin builder defaults to the config
              effectiveCfg = pluginsLib.applyBuilderDefaults loadedPlugins.builders cfg;

              # Resolve the workspace root for this package:
              # if it came from a subworkspace, use that subworkspace's root.
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
                builtins.elem system (cfg.systems or systems)
            )
            packageConfigs
          );

        # Collect extra shell packages from loaded plugins
        pluginShellExtras = pluginsLib.collectShellExtras loadedPlugins.shellExtras pkgs;

        # Build dev shells for this system
        builtShells =
          lib.mapAttrs (
            name: cfg:
              shellBuilder.buildShell pkgs name cfg builtPackages pluginShellExtras
          )
          (
            lib.filterAttrs (
              _: cfg:
                builtins.elem system (cfg.systems or systems)
            )
            shellConfigs
          );

        # If there's exactly one package and no explicit default shell,
        # create a default shell with that package's build inputs.
        hasDefaultShell = builtShells ? default;
        packageNames = builtins.attrNames builtPackages;
        autoDefaultShell =
          if !hasDefaultShell && builtins.length packageNames == 1
          then let
            singlePkgName = builtins.head packageNames;
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
      # For each subworkspace, compute its namespaced output names
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
      builtins.listToAttrs subEntries;

    resolvePackageRoot = pkgName:
      subworkspaceOutputRoots.${pkgName} or workspaceRoot;

    # ── Non-per-system outputs (nixosConfigurations, modules) ───
    #
    # NixOS configurations are not per-system in the flake output schema —
    # each configuration declares its own system internally.

    # Merge discovered .nix files with any paths declared in Nickel module configs.
    # For root workspace modules:
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

    # Also discover and resolve module paths from subworkspaces
    subworkspaceModulePaths = let
      allSubModulePaths = lib.concatLists (
        lib.mapAttrsToList (
          subName: subInfo: let
            subRoot = subInfo.path;
            subNixFiles = moduleBuilder.discoverNixFiles subRoot "modules";
            # Namespace the module names
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
      builtins.listToAttrs allSubModulePaths;

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
      builtins.listToAttrs allSubHomePaths;

    # Combine root and subworkspace module paths
    allModulePaths = resolvedModulePaths // subworkspaceModulePaths;
    allHomePaths = resolvedHomePaths // subworkspaceHomePaths;

    # Build NixOS machine configurations
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

    # Build NixOS module flake outputs
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

    # Build home-manager module flake outputs
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

    # Build overlay flake outputs
    overlays =
      if overlayConfigs != {}
      then
        overlayBuilder.buildAllOverlays {
          inherit workspaceRoot overlayConfigs;
        }
      else {};

    # Build template flake outputs
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
      # Expose loaded plugin metadata for debugging/introspection
      _pluginMeta =
        lib.mapAttrs (
          _name: cfg: {
            name = cfg.name or "unknown";
            description = cfg.description or "";
            conventions = builtins.attrNames (cfg.conventions or {});
          }
        )
        evaluatedPluginConfigs;
    });
in {
  inherit mkWorkspace;

  # Re-export sub-modules for advanced usage / testing
  inherit discover systemsLib namespacingLib packageBuilder shellBuilder machineBuilder moduleBuilder evalNickel;
}
