# The bridge from the Nickel validation layer to the Nix builder layer: a
# generated wrapper .ncl imports the discovered files and applies the contracts,
# `nickel export` turns that into JSON, and the JSON is read back through one
# import-from-derivation. Every contract check happens inside the Nickel
# evaluation, so the Nix side only ever sees a validated configuration tree.
{lib}: let
  # One field of the discovered record:
  #   "hello" = import "/nix/store/.../packages/hello.ncl",
  mkImportField = name: path: ''"${name}" = import "${toString path}",'';

  mkImportBlock = entries:
    lib.concatStringsSep "\n" (lib.mapAttrsToList mkImportField entries);

  # ── Plugin-aware wrapper generation ─────────────────────────────
  #
  # Each plugin is imported, validated against PluginConfig, and its contract
  # extensions merged into PackageConfig and ShellConfig. workspace.ncl exports
  # `mkWorkspaceConfig`, a factory taking (pkg_contract, shell_contract), which
  # is what lets the extended sub-contracts be swapped in without restating the
  # workspace structure. With no plugins, `WorkspaceConfig` - the factory
  # applied to the base contracts - is used directly.
  mkPluginPreamble = contractsDir: pluginNclPaths: let
    indexed =
      lib.imap0 (i: path: {
        varName = "plugin_${toString i}";
        inherit path;
      })
      pluginNclPaths;

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

    # Each plugin's extend.<ContractName>, merged in, or {} where it has none.
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

  # The wrapper imports the contracts and every discovered .ncl, merges the
  # user's workspace.ncl over that, and applies the workspace contract.
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

  # generateWrapperSource against a subworkspace: its own workspace.ncl and
  # convention directories, but the root workspace's plugin extensions, so the
  # contracts are the same ones. The result is an independent config tree, and
  # namespacing happens on the Nix side afterwards.
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

  # Run Nickel over a workspace and read the validated config back as an
  # attribute set. The `discovered*` arguments are { name = /path/to/name.ncl; }.
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

    # writeTextFile tracks the store paths named in the text, which is what puts
    # the contracts and workspace sources in the sandbox Nickel imports from.
    wrapperFile = bootstrapPkgs.writeTextFile {
      name = "nix-workspace-eval.ncl";
      text = wrapperSource;
    };

    pluginDirRefs = map (p: builtins.dirOf p) pluginNclPaths;

    evalDrv =
      bootstrapPkgs.runCommand "nix-workspace-eval" (
        {
          nativeBuildInputs = [bootstrapPkgs.nickel];

          # wrapperFile already names these textually; stating them again is
          # belt and braces for the sandbox.
          inherit contractsDir workspaceRoot;
        }
        // (lib.optionalAttrs (pluginNclPaths != []) {
          pluginDirs = pluginDirRefs;
        })
      ) ''
        nickel export ${wrapperFile} > $out
      '';
  in
    builtins.fromJSON (builtins.readFile evalDrv);

  # evalWorkspace for one subworkspace, producing an independent validated
  # config tree that the caller then namespaces.
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

  # `subworkspaceMap` is discover.discoverAllSubworkspaces' output.
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
  # A plugin's conventions, contracts and extensions, which is where the Nix
  # side gets its convention mappings and builder metadata from.
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

  # The fallback when there is no workspace.ncl and nothing was discovered:
  # Nickel is skipped entirely and outputs come from the Nix-side config alone.
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
