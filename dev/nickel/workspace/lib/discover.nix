# Directory auto-discovery for nix-workspace
#
# Scans convention directories (packages/, shells/, etc.) for .ncl files
# and maps them to output names.
#
# Convention:
#   packages/hello.ncl      → packages.hello
#   packages/default.ncl    → packages.<workspace-name> (in subworkspaces)
#   shells/default.ncl      → devShells.default
#
# Updated for v0.3 to support recursive subworkspace discovery.
# A subworkspace is any subdirectory containing a `workspace.ncl` file.
# Discovery is VCS-agnostic — it does not parse `.gitmodules` or any
# VCS metadata. As long as the directory exists and contains
# `workspace.ncl`, it participates in the workspace.
#
{lib}: let
  # Default convention directory mappings
  # Maps convention name → { dir, output }
  defaultConventions = {
    packages = {
      dir = "packages";
      output = "packages";
    };
    shells = {
      dir = "shells";
      output = "devShells";
    };
    modules = {
      dir = "modules";
      output = "nixosModules";
    };
    home = {
      dir = "home";
      output = "homeModules";
    };
    overlays = {
      dir = "overlays";
      output = "overlays";
    };
    machines = {
      dir = "machines";
      output = "nixosConfigurations";
    };
    templates = {
      dir = "templates";
      output = "templates";
    };
    checks = {
      dir = "checks";
      output = "checks";
    };
    lib = {
      dir = "lib";
      output = "lib";
    };
  };

  # Apply user convention overrides to the defaults.
  # Users can change directory paths or disable auto-discovery.
  applyConventionOverrides = conventions: overrides:
    lib.mapAttrs (
      name: conv:
        if builtins.hasAttr name overrides
        then let
          ovr = overrides.${name};
        in
          conv
          // (lib.optionalAttrs (ovr ? path) {dir = ovr.path;})
          // (lib.optionalAttrs (ovr ? auto-discover) {autoDiscover = ovr.auto-discover;})
        else conv // {autoDiscover = true;}
    )
    conventions;

  # List .ncl files in a directory and return a name → path mapping.
  #
  # Given workspaceRoot and a relative directory:
  #   packages/hello.ncl  →  { hello = /absolute/path/packages/hello.ncl; }
  #   packages/default.ncl → { default = /absolute/path/packages/default.ncl; }
  #
  discoverNclFiles = workspaceRoot: relativeDir: let
    dirPath = workspaceRoot + "/${relativeDir}";
  in
    if builtins.pathExists dirPath
    then let
      entries = builtins.readDir dirPath;
      nclEntries =
        lib.filterAttrs (
          name: type:
            type == "regular" && lib.hasSuffix ".ncl" name
        )
        entries;
    in
      lib.mapAttrs' (
        name: _: {
          name = lib.removeSuffix ".ncl" name;
          value = dirPath + "/${name}";
        }
      )
      nclEntries
    else {};

  # Check if a directory exists within the workspace root
  dirExists = workspaceRoot: relativeDir:
    builtins.pathExists (workspaceRoot + "/${relativeDir}");

  # Discover all convention directories and their .ncl files.
  #
  # Returns:
  # {
  #   packages = { hello = /path/to/packages/hello.ncl; ... };
  #   shells = { default = /path/to/shells/default.ncl; ... };
  #   ...
  # }
  #
  discoverAll = workspaceRoot: conventionOverrides: let
    conventions = applyConventionOverrides defaultConventions (
      if conventionOverrides == null
      then {}
      else conventionOverrides
    );
    activeConventions = lib.filterAttrs (_: conv: conv.autoDiscover or true) conventions;
  in
    lib.mapAttrs (
      _name: conv:
        discoverNclFiles workspaceRoot conv.dir
    )
    activeConventions;

  # ── Subworkspace discovery ──────────────────────────────────────
  #
  # Subworkspaces are discovered by scanning the workspace root for
  # subdirectories that contain a `workspace.ncl` file. This is
  # VCS-agnostic — git submodules, jujutsu checkouts, plain dirs,
  # and symlinks all work identically.

  # Discover subworkspaces: subdirectories that contain a workspace.ncl file.
  #
  # Returns: { name = path; ... } where name is the directory name
  # and path is the absolute path to the subworkspace root.
  #
  # Skips hidden directories (starting with ".") and well-known
  # non-workspace directories (node_modules, .git, result, etc.)
  #
  discoverSubworkspaces = workspaceRoot: let
    entries =
      if builtins.pathExists workspaceRoot
      then builtins.readDir workspaceRoot
      else {};

    # Filter to directories only, excluding hidden dirs and known non-workspace dirs
    skipDirs = [".git" ".github" ".gitlab" "node_modules" "result" ".direnv" ".devenv"];

    dirs =
      lib.filterAttrs (
        name: type:
          (type == "directory" || type == "symlink")
          && !(lib.hasPrefix "." name && builtins.elem name skipDirs)
          # Also skip convention directories — they are not subworkspaces
          && !(builtins.elem name (map (c: c.dir) (builtins.attrValues defaultConventions)))
      )
      entries;
  in
    lib.filterAttrs (
      name: _:
        builtins.pathExists (workspaceRoot + "/${name}/workspace.ncl")
    ) (
      lib.mapAttrs (
        name: _: workspaceRoot + "/${name}"
      )
      dirs
    );

  # Discover all convention outputs for a single subworkspace.
  #
  # Type: Path -> AttrSet -> AttrSet
  #
  # Arguments:
  #   subworkspaceRoot    — Absolute path to the subworkspace directory
  #   conventionOverrides — Convention overrides from the subworkspace's config (or null)
  #
  # Returns:
  #   {
  #     packages = { default = /path/to/packages/default.ncl; lsp = ...; };
  #     shells = { ... };
  #     ...
  #   }
  #
  discoverSubworkspaceOutputs = subworkspaceRoot: conventionOverrides:
    discoverAll subworkspaceRoot conventionOverrides;

  # Full subworkspace discovery: find all subworkspaces and scan their contents.
  #
  # Type: Path -> AttrSet
  #
  # Arguments:
  #   workspaceRoot — Absolute path to the root workspace
  #
  # Returns:
  #   {
  #     <dir-name> = {
  #       path = /absolute/path/to/subworkspace;
  #       hasWorkspaceNcl = true;
  #       discovered = {
  #         packages = { ... };
  #         shells = { ... };
  #         ...
  #       };
  #     };
  #     ...
  #   }
  #
  discoverAllSubworkspaces = workspaceRoot: let
    subworkspaces = discoverSubworkspaces workspaceRoot;
  in
    lib.mapAttrs (
      _name: path: {
        inherit path;
        hasWorkspaceNcl = true; # by definition — we only discover dirs with workspace.ncl
        discovered = discoverSubworkspaceOutputs path null;
      }
    )
    subworkspaces;

  # Generate a mapping of discovered output names.
  #
  # Resolves "default" entries for subworkspaces:
  #   In a subworkspace named "foo", packages/default.ncl → "foo"
  #   Named entries get prefixed: packages/bar.ncl → "foo-bar"
  #
  resolveNames = {
    workspaceName ? null,
    isSubworkspace ? false,
  }: discovered:
    lib.mapAttrs (
      _conventionName: files:
        lib.listToAttrs (
          lib.mapAttrsToList (
            fileName: filePath: let
              outputName =
                if isSubworkspace && workspaceName != null
                then
                  if fileName == "default"
                  then workspaceName
                  else "${workspaceName}-${fileName}"
                else fileName;
            in {
              name = outputName;
              value = filePath;
            }
          )
          files
        )
    )
    discovered;

  # Apply namespacing to discovered subworkspace outputs.
  #
  # Type: String -> AttrSet -> AttrSet
  #
  # Given a subworkspace directory name and its raw discovered outputs,
  # returns the same structure with output names namespaced:
  #   default   → subworkspaceName
  #   otherName → subworkspaceName-otherName
  #
  namespaceSubworkspaceDiscovered = subworkspaceName: discovered:
    resolveNames {
      workspaceName = subworkspaceName;
      isSubworkspace = true;
    }
    discovered;

  # Merge root and subworkspace discovered outputs into a single tree.
  #
  # Type: AttrSet -> AttrSet -> AttrSet
  #
  # Arguments:
  #   rootDiscovered      — { convention = { name = path; }; ... } from root workspace
  #   subworkspaceMap     — Output of discoverAllSubworkspaces
  #
  # Returns:
  #   {
  #     merged = { convention = { name = path; }; ... };
  #     subworkspaceNames = [ "sub-a" "sub-b" ];
  #     subworkspaceInfo = { name = { path, discovered, namespaced }; ... };
  #   }
  #
  mergeDiscovered = rootDiscovered: subworkspaceMap: let
    subNames = builtins.attrNames subworkspaceMap;

    # Namespace each subworkspace's discovered outputs
    namespacedSubs =
      lib.mapAttrs (
        name: info:
          info
          // {
            namespaced = namespaceSubworkspaceDiscovered name info.discovered;
          }
      )
      subworkspaceMap;

    # Merge all namespaced subworkspace outputs into the root
    merged =
      builtins.foldl' (
        acc: subName: let
          sub = namespacedSubs.${subName};
          subOutputs = sub.namespaced;
        in
          lib.mapAttrs (
            convention: rootOutputs: let
              subConvOutputs = subOutputs.${convention} or {};
            in
              rootOutputs // subConvOutputs
          )
          acc
      )
      rootDiscovered
      subNames;
  in {
    inherit merged;
    subworkspaceNames = subNames;
    subworkspaceInfo = namespacedSubs;
  };

  # Check for naming conflicts between root outputs and subworkspace outputs,
  # and between subworkspaces.
  #
  # Type: AttrSet -> AttrSet -> [AttrSet]
  #
  # Arguments:
  #   rootDiscovered  — Root workspace discovered outputs
  #   subworkspaceMap — Output of discoverAllSubworkspaces
  #
  # Returns: List of conflict diagnostic records
  #
  checkDiscoveryConflicts = rootDiscovered: subworkspaceMap: let
    subNames = builtins.attrNames subworkspaceMap;

    # Build a registry: { convention.outputName = ["source1", "source2"] }
    # First, register root outputs
    rootRegistry =
      lib.mapAttrs (
        _convention: outputs:
          lib.mapAttrs (_name: _: ["root"]) outputs
      )
      rootDiscovered;

    # Then, for each subworkspace, namespace its outputs and register them
    registryWithSubs =
      builtins.foldl' (
        registry: subName: let
          subInfo = subworkspaceMap.${subName};
          namespacedOutputs = namespaceSubworkspaceDiscovered subName subInfo.discovered;
        in
          lib.mapAttrs (
            convention: existingNames: let
              subConv = namespacedOutputs.${convention} or {};
            in
              lib.foldlAttrs (
                acc: name: _value: let
                  existing = acc.${name} or [];
                in
                  acc // {${name} = existing ++ ["subworkspace:${subName}"];}
              )
              existingNames
              subConv
          )
          registry
      )
      rootRegistry
      subNames;

    # Find entries with multiple sources
    conflicts = lib.concatLists (
      lib.mapAttrsToList (
        convention: names:
          lib.concatLists (
            lib.mapAttrsToList (
              name: sources:
                if builtins.length sources > 1
                then [
                  {
                    code = "NW200";
                    severity = "error";
                    inherit convention name sources;
                    message = "Namespace conflict: output '${name}' in '${convention}' is produced by ${builtins.toString (builtins.length sources)} sources: ${builtins.concatStringsSep ", " sources}";
                    hint = "Rename one of the conflicting outputs or use a different subworkspace directory name.";
                  }
                ]
                else []
            )
            names
          )
      )
      registryWithSubs
    );
  in
    conflicts;
in {
  inherit
    defaultConventions
    applyConventionOverrides
    discoverNclFiles
    discoverAll
    discoverSubworkspaces
    discoverSubworkspaceOutputs
    discoverAllSubworkspaces
    resolveNames
    namespaceSubworkspaceDiscovered
    mergeDiscovered
    checkDiscoveryConflicts
    dirExists
    ;
}
