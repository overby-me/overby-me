# eval-nickel.nix — Evaluate workspace.ncl via Nickel and import the result as JSON
#
# Uses Import From Derivation (IFD) to bridge Nickel → Nix:
#   1. Generate a wrapper .ncl that imports discovered files and applies contracts
#   2. Run `nickel export` to produce JSON
#   3. Read the JSON back into Nix
#
# This module is the bridge between the Nickel validation layer and the Nix
# builder layer. All contract checking happens inside the Nickel evaluation;
# the Nix side receives a fully validated configuration tree.
#
# Updated for v0.4 to support plugin-aware evaluation. When plugins are
# loaded, the wrapper Nickel code:
#   1. Imports each plugin's plugin.ncl and validates it against PluginConfig
#   2. Merges plugin contract extensions into extended PackageConfig/ShellConfig
#   3. Uses `mkWorkspaceConfig ExtPkg ExtShell` to produce a workspace contract
#      that validates plugin-specific fields
#   4. Applies the extended contract to the merged workspace config
#
# Without plugins, the wrapper uses `WorkspaceConfig` directly (which is
# defined as `mkWorkspaceConfig PackageConfig ShellConfig`), producing
# identical behavior to pre-v0.4.
{lib}: let
  # ── Helpers ─────────────────────────────────────────────────────
  # Generate the Nickel source for a single discovered entry.
  # Each entry becomes a field in the discovered record.
  #   e.g. { hello = import "/nix/store/.../packages/hello.ncl" }
  mkImportField = name: path: ''"${name}" = import "${toString path}",'';

  # Generate a block of Nickel import fields for a convention directory.
  mkImportBlock = entries:
    lib.concatStringsSep "\n" (lib.mapAttrsToList mkImportField entries);

  # ── Plugin-aware wrapper generation ─────────────────────────────
  #
  # When plugins are loaded, the wrapper:
  #   1. Imports PluginConfig contract
  #   2. Imports and validates each plugin's plugin.ncl
  #   3. Merges all plugin extensions for PackageConfig and ShellConfig
  #   4. Calls mkWorkspaceConfig with the extended sub-contracts
  #   5. Applies the resulting contract to the merged config
  #
  # When no plugins are loaded, it simply applies WorkspaceConfig directly.
  #
  # The key design insight: workspace.ncl exports `mkWorkspaceConfig`,
  # a factory function that takes (pkg_contract, shell_contract) and
  # returns a full workspace config contract. This lets us swap in
  # extended sub-contracts without duplicating the workspace structure.

  # Generate Nickel source for plugin imports and contract extension.
  #
  # Type: Path -> [Path] -> String
  #
  # Arguments:
  #   contractsDir   — Path to nix-workspace contracts/
  #   pluginNclPaths — List of plugin .ncl file paths
  #
  # Returns: A Nickel code fragment with:
  #   - let plugin_0 = ... in
  #   - let plugin_1 = ... in
  #   - let ExtPkg = PackageConfig & ext0 & ext1 in
  #   - let ExtShell = ShellConfig & ext0 & ext1 in
  #   - let EffectiveWorkspaceConfig = mkWorkspaceConfig ExtPkg ExtShell in
  #
  mkPluginPreamble = contractsDir: pluginNclPaths: let
    # Generate plugin variable names: plugin_0, plugin_1, ...
    indexed =
      lib.imap0 (i: path: {
        varName = "plugin_${toString i}";
        inherit path;
      })
      pluginNclPaths;

    # Import and validate each plugin
    pluginImports =
      lib.concatMapStringsSep "" (
        entry: ''
          let ${entry.varName} =
            let { PluginConfig, .. } = import "${toString contractsDir}/plugin.ncl" in
            (import "${toString entry.path}") | PluginConfig
          in
        ''
      )
      indexed;

    # Build the extension chain for a given contract name.
    # Each plugin's extend.<ContractName> is merged in (defaulting to {} if absent).
    mkExtChain = contractName: baseContract: let
      extExprs =
        map (
          entry: ''(if std.record.has_field "${contractName}" ${entry.varName}.extend then ${entry.varName}.extend."${contractName}" else {})''
        )
        indexed;
    in
      if indexed == []
      then baseContract
      else "${baseContract} & ${lib.concatStringsSep " & " extExprs}";

    extPkgExpr = mkExtChain "PackageConfig" "PackageConfig";
    extShellExpr = mkExtChain "ShellConfig" "ShellConfig";
  in
    pluginImports
    + ''
      let ExtPkg = ${extPkgExpr} in
      let ExtShell = ${extShellExpr} in
      let EffectiveWorkspaceConfig = mkWorkspaceConfig ExtPkg ExtShell in
    '';

  # ── Wrapper generation ──────────────────────────────────────────

  # Build the wrapper .ncl source that ties everything together.
  #
  # The wrapper:
  #   1. Imports contracts from the nix-workspace contracts/ directory
  #   2. Imports discovered .ncl files from convention directories
  #   3. Imports the user's workspace.ncl (if present)
  #   4. Merges discovered config with workspace config
  #   5. Applies the (possibly extended) WorkspaceConfig contract
  #
  # When pluginNclPaths is non-empty, it uses mkWorkspaceConfig with
  # extended sub-contracts. When empty, it uses WorkspaceConfig directly.
  generateWrapperSource = {
    contractsDir,
    workspaceRoot,
    discoveredPackages ? {},
    discoveredShells ? {},
    discoveredMachines ? {},
    discoveredModules ? {},
    discoveredHome ? {},
    discoveredOverlays ? {},
    discoveredChecks ? {},
    discoveredTemplates ? {},
    hasWorkspaceNcl ? false,
    pluginNclPaths ? [],
  }: let
    packageFields = mkImportBlock discoveredPackages;
    shellFields = mkImportBlock discoveredShells;
    machineFields = mkImportBlock discoveredMachines;
    moduleFields = mkImportBlock discoveredModules;
    homeFields = mkImportBlock discoveredHome;
    overlayFields = mkImportBlock discoveredOverlays;
    checkFields = mkImportBlock discoveredChecks;
    templateFields = mkImportBlock discoveredTemplates;

    hasPlugins = pluginNclPaths != [];

    # Plugin preamble: imports, validation, extension chains, effective contract
    pluginPreamble =
      if hasPlugins
      then mkPluginPreamble contractsDir pluginNclPaths
      else "";

    # The contract to apply at the end
    finalContract =
      if hasPlugins
      then "EffectiveWorkspaceConfig"
      else "WorkspaceConfig";

    # If workspace.ncl exists, import and merge it; otherwise use empty record
    workspaceMerge =
      if hasWorkspaceNcl
      then ''
        let workspace_config = import "${toString workspaceRoot}/workspace.ncl" in
        (discovered & workspace_config)
      ''
      else "discovered";
  in ''
    let { WorkspaceConfig, mkWorkspaceConfig, .. } = import "${toString contractsDir}/workspace.ncl" in
    let { PackageConfig, .. } = import "${toString contractsDir}/package.ncl" in
    let { ShellConfig, .. } = import "${toString contractsDir}/shell.ncl" in
    ${pluginPreamble}let discovered = {
      packages = {
    ${packageFields}
      },
      shells = {
    ${shellFields}
      },
      machines = {
    ${machineFields}
      },
      modules = {
    ${moduleFields}
      },
      home = {
    ${homeFields}
      },
      overlays = {
    ${overlayFields}
      },
      checks = {
    ${checkFields}
      },
      templates = {
    ${templateFields}
      },
    } in
    (${lib.strings.trim workspaceMerge}) | ${finalContract}
  '';

  # Generate a wrapper .ncl source for a subworkspace.
  #
  # This is similar to generateWrapperSource but tailored for subworkspaces:
  #   - Uses the subworkspace's own workspace.ncl
  #   - Discovers from the subworkspace's convention directories
  #   - Applies the same (possibly extended) contract
  #
  # Plugin extensions from the root workspace are propagated to subworkspaces
  # so they benefit from the same extended contracts.
  #
  # The result is an independent config tree. Namespacing is applied
  # on the Nix side after evaluation.
  generateSubworkspaceWrapperSource = {
    contractsDir,
    subworkspaceRoot,
    subworkspaceName,
    discoveredPackages ? {},
    discoveredShells ? {},
    discoveredMachines ? {},
    discoveredModules ? {},
    discoveredHome ? {},
    discoveredOverlays ? {},
    discoveredChecks ? {},
    discoveredTemplates ? {},
    hasWorkspaceNcl ? true,
    pluginNclPaths ? [],
  }: let
    packageFields = mkImportBlock discoveredPackages;
    shellFields = mkImportBlock discoveredShells;
    machineFields = mkImportBlock discoveredMachines;
    moduleFields = mkImportBlock discoveredModules;
    homeFields = mkImportBlock discoveredHome;
    overlayFields = mkImportBlock discoveredOverlays;
    checkFields = mkImportBlock discoveredChecks;
    templateFields = mkImportBlock discoveredTemplates;

    hasPlugins = pluginNclPaths != [];

    pluginPreamble =
      if hasPlugins
      then mkPluginPreamble contractsDir pluginNclPaths
      else "";

    finalContract =
      if hasPlugins
      then "EffectiveWorkspaceConfig"
      else "WorkspaceConfig";

    workspaceMerge =
      if hasWorkspaceNcl
      then ''
        let workspace_config = import "${toString subworkspaceRoot}/workspace.ncl" in
        (discovered & workspace_config)
      ''
      else ''
        (discovered & { name = "${subworkspaceName}" })
      '';
  in ''
    let { WorkspaceConfig, mkWorkspaceConfig, .. } = import "${toString contractsDir}/workspace.ncl" in
    let { PackageConfig, .. } = import "${toString contractsDir}/package.ncl" in
    let { ShellConfig, .. } = import "${toString contractsDir}/shell.ncl" in
    ${pluginPreamble}let discovered = {
      packages = {
    ${packageFields}
      },
      shells = {
    ${shellFields}
      },
      machines = {
    ${machineFields}
      },
      modules = {
    ${moduleFields}
      },
      home = {
    ${homeFields}
      },
      overlays = {
    ${overlayFields}
      },
      checks = {
    ${checkFields}
      },
      templates = {
    ${templateFields}
      },
    } in
    (${lib.strings.trim workspaceMerge}) | ${finalContract}
  '';

  # ── Evaluation functions ────────────────────────────────────────

  # Evaluate a workspace by running Nickel and reading back JSON via IFD.
  #
  # Arguments:
  #   bootstrapPkgs      — A nixpkgs package set used to obtain the `nickel` binary
  #   contractsDir       — Path to the nix-workspace contracts/ directory
  #   workspaceRoot      — Path to the user's workspace root
  #   discoveredPackages — Attrset of { name = /path/to/name.ncl; ... }
  #   discoveredShells   — Attrset of { name = /path/to/name.ncl; ... }
  #   discoveredMachines — Attrset of { name = /path/to/name.ncl; ... }
  #   discoveredModules  — Attrset of { name = /path/to/name.ncl; ... }
  #   discoveredHome     — Attrset of { name = /path/to/name.ncl; ... }
  #   pluginNclPaths     — List of paths to plugin .ncl files (optional, default [])
  #
  # Returns: An attribute set (the validated workspace configuration).
  evalWorkspace = {
    bootstrapPkgs,
    contractsDir,
    workspaceRoot,
    discoveredPackages ? {},
    discoveredShells ? {},
    discoveredMachines ? {},
    discoveredModules ? {},
    discoveredHome ? {},
    discoveredOverlays ? {},
    discoveredChecks ? {},
    discoveredTemplates ? {},
    pluginNclPaths ? [],
  }: let
    hasWorkspaceNcl = builtins.pathExists (workspaceRoot + "/workspace.ncl");

    wrapperSource = generateWrapperSource {
      inherit
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
        hasWorkspaceNcl
        pluginNclPaths
        ;
    };

    # Write the generated wrapper to the store so Nickel can import from it.
    # writeTextFile properly tracks store-path references in the text,
    # ensuring the sandbox has access to contracts and workspace sources.
    wrapperFile = bootstrapPkgs.writeTextFile {
      name = "nix-workspace-eval.ncl";
      text = wrapperSource;
    };

    # Collect plugin directories for sandbox access
    pluginDirRefs = map (p: builtins.dirOf p) pluginNclPaths;

    # Run nickel export inside a derivation (IFD).
    # The output is a single JSON file representing the validated config.
    evalDrv =
      bootstrapPkgs.runCommand "nix-workspace-eval" (
        {
          nativeBuildInputs = [bootstrapPkgs.nickel];

          # Explicitly reference source paths so they appear in the build sandbox.
          # Even though wrapperFile already references them textually, being
          # explicit avoids any edge-case sandbox issues.
          inherit contractsDir workspaceRoot;
        }
        // (lib.optionalAttrs (pluginNclPaths != []) {
          # Reference plugin directories so they are available in the sandbox
          pluginDirs = pluginDirRefs;
        })
      ) ''
        nickel export ${wrapperFile} > $out
      '';
  in
    builtins.fromJSON (builtins.readFile evalDrv);

  # Evaluate a subworkspace's workspace.ncl through Nickel.
  #
  # This is the subworkspace counterpart of evalWorkspace. It produces
  # an independent validated config tree for a single subworkspace.
  # The caller is responsible for namespacing the outputs after evaluation.
  #
  # Plugin extensions from the root workspace are propagated to subworkspaces
  # so they benefit from the same extended contracts.
  #
  # Arguments:
  #   bootstrapPkgs      — A nixpkgs package set (for nickel binary)
  #   contractsDir       — Path to nix-workspace contracts/
  #   subworkspaceRoot   — Absolute path to the subworkspace directory
  #   subworkspaceName   — Directory name of the subworkspace (e.g. "mojo-zed")
  #   discoveredPackages  — { name = path; ... } from subworkspace discovery
  #   discoveredShells    — { name = path; ... }
  #   discoveredMachines  — { name = path; ... }
  #   discoveredModules   — { name = path; ... }
  #   discoveredHome      — { name = path; ... }
  #   discoveredOverlays  — { name = path; ... }
  #   discoveredChecks    — { name = path; ... }
  #   discoveredTemplates — { name = path; ... }
  #   pluginNclPaths      — Plugin .ncl paths from the root workspace (optional)
  #
  # Returns: An attribute set (the validated subworkspace configuration).
  evalSubworkspace = {
    bootstrapPkgs,
    contractsDir,
    subworkspaceRoot,
    subworkspaceName,
    discoveredPackages ? {},
    discoveredShells ? {},
    discoveredMachines ? {},
    discoveredModules ? {},
    discoveredHome ? {},
    discoveredOverlays ? {},
    discoveredChecks ? {},
    discoveredTemplates ? {},
    pluginNclPaths ? [],
  }: let
    hasWorkspaceNcl = builtins.pathExists (subworkspaceRoot + "/workspace.ncl");

    wrapperSource = generateSubworkspaceWrapperSource {
      inherit
        contractsDir
        subworkspaceRoot
        subworkspaceName
        discoveredPackages
        discoveredShells
        discoveredMachines
        discoveredModules
        discoveredHome
        discoveredOverlays
        discoveredChecks
        discoveredTemplates
        hasWorkspaceNcl
        pluginNclPaths
        ;
    };

    wrapperFile = bootstrapPkgs.writeTextFile {
      name = "nix-workspace-eval-${subworkspaceName}.ncl";
      text = wrapperSource;
    };

    pluginDirRefs = map (p: builtins.dirOf p) pluginNclPaths;

    evalDrv =
      bootstrapPkgs.runCommand "nix-workspace-eval-${subworkspaceName}" (
        {
          nativeBuildInputs = [bootstrapPkgs.nickel];
          inherit contractsDir subworkspaceRoot;
        }
        // (lib.optionalAttrs (pluginNclPaths != []) {
          pluginDirs = pluginDirRefs;
        })
      ) ''
        nickel export ${wrapperFile} > $out
      '';
  in
    builtins.fromJSON (builtins.readFile evalDrv);

  # Evaluate all subworkspaces discovered in a workspace.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   bootstrapPkgs    — A nixpkgs package set (for nickel binary)
  #   contractsDir     — Path to nix-workspace contracts/
  #   subworkspaceMap  — Output of discover.discoverAllSubworkspaces
  #                      { name = { path, discovered, hasWorkspaceNcl }; ... }
  #   pluginNclPaths   — Plugin .ncl paths from the root workspace (optional)
  #
  # Returns:
  #   { name = evaluatedConfig; ... }
  #   where each evaluatedConfig is the full validated workspace config
  #   for that subworkspace.
  evalAllSubworkspaces = {
    bootstrapPkgs,
    contractsDir,
    subworkspaceMap,
    pluginNclPaths ? [],
  }:
    lib.mapAttrs (
      name: info: let
        inherit (info) discovered;
      in
        evalSubworkspace {
          inherit bootstrapPkgs contractsDir pluginNclPaths;
          subworkspaceRoot = info.path;
          subworkspaceName = name;
          discoveredPackages = discovered.packages or {};
          discoveredShells = discovered.shells or {};
          discoveredMachines = discovered.machines or {};
          discoveredModules = discovered.modules or {};
          discoveredHome = discovered.home or {};
          discoveredOverlays = discovered.overlays or {};
          discoveredChecks = discovered.checks or {};
          discoveredTemplates = discovered.templates or {};
        }
    )
    subworkspaceMap;

  # ── Plugin evaluation ───────────────────────────────────────────
  #
  # Evaluate a plugin's plugin.ncl through Nickel to obtain its
  # configuration (conventions, contracts, extensions, etc.) as JSON.
  #
  # This is used by the Nix-side plugin system to extract convention
  # mappings and builder metadata from plugin definitions.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   bootstrapPkgs — A nixpkgs package set (for nickel binary)
  #   contractsDir  — Path to nix-workspace contracts/
  #   pluginNclPath — Path to the plugin's plugin.ncl file
  #   pluginName    — Plugin name (for derivation naming)
  #
  # Returns: The evaluated plugin config as a Nix attribute set.
  evalPlugin = {
    bootstrapPkgs,
    contractsDir,
    pluginNclPath,
    pluginName,
  }: let
    wrapperSource = ''
      let { PluginConfig, .. } = import "${toString contractsDir}/plugin.ncl" in
      (import "${toString pluginNclPath}") | PluginConfig
    '';

    wrapperFile = bootstrapPkgs.writeTextFile {
      name = "nix-workspace-plugin-eval-${pluginName}.ncl";
      text = wrapperSource;
    };

    evalDrv =
      bootstrapPkgs.runCommand "nix-workspace-plugin-eval-${pluginName}" {
        nativeBuildInputs = [bootstrapPkgs.nickel];
        inherit contractsDir;
        pluginDir = builtins.dirOf pluginNclPath;
      } ''
        nickel export ${wrapperFile} > $out
      '';
  in
    builtins.fromJSON (builtins.readFile evalDrv);

  # Evaluate all plugins referenced in the workspace config.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   bootstrapPkgs   — A nixpkgs package set (for nickel binary)
  #   contractsDir    — Path to nix-workspace contracts/
  #   pluginNclPaths  — { pluginName = /path/to/plugin.ncl; ... }
  #
  # Returns:
  #   { pluginName = evaluatedPluginConfig; ... }
  evalAllPlugins = {
    bootstrapPkgs,
    contractsDir,
    pluginNclPaths,
  }:
    lib.mapAttrs (
      pluginName: pluginNclPath:
        evalPlugin {
          inherit bootstrapPkgs contractsDir pluginNclPath pluginName;
        }
    )
    pluginNclPaths;

  # Light-weight evaluation: skip Nickel entirely and return a minimal
  # default config. Used as a fallback when there is no workspace.ncl
  # and no discovered .ncl files, so we can still produce outputs from
  # the Nix-side config alone.
  emptyConfig = {
    name = "unnamed";
    systems = ["x86_64-linux" "aarch64-linux"];
    nixpkgs = {};
    packages = {};
    shells = {};
    machines = {};
    modules = {};
    home = {};
    overlays = {};
    checks = {};
    templates = {};
    conventions = {};
    dependencies = {};
    plugins = [];
  };
in {
  inherit
    evalWorkspace
    evalSubworkspace
    evalAllSubworkspaces
    evalPlugin
    evalAllPlugins
    generateWrapperSource
    generateSubworkspaceWrapperSource
    emptyConfig
    ;
}
