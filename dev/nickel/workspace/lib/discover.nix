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
# A subworkspace is any subdirectory containing a `workspace.ncl` file.
# Discovery is VCS-agnostic — it does not parse `.gitmodules` or any
# VCS metadata. As long as the directory exists and contains
# `workspace.ncl`, it participates in the workspace.
{lib}: let
  # convention name → { dir, output }
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

  # A user may change a directory path or turn auto-discovery off.
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

  # Relative to workspaceRoot:
  #   packages/hello.ncl   → { hello = /absolute/path/packages/hello.ncl; }
  #   packages/default.ncl → { default = /absolute/path/packages/default.ncl; }
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

  dirExists = workspaceRoot: relativeDir:
    builtins.pathExists (workspaceRoot + "/${relativeDir}");

  # Every convention directory, as
  #   { packages = { hello = /path/...; }; shells = { default = /path/...; }; }
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

  # { directory name = absolute path; ... } for each subdirectory holding a
  # workspace.ncl. Hidden directories and the well-known non-workspace ones
  # (node_modules, .git, result) are skipped.
  discoverSubworkspaces = workspaceRoot: let
    entries =
      if builtins.pathExists workspaceRoot
      then builtins.readDir workspaceRoot
      else {};

    skipDirs = [".git" ".github" ".gitlab" "node_modules" "result" ".direnv" ".devenv"];

    dirs =
      lib.filterAttrs (
        name: type:
          (type == "directory" || type == "symlink")
          && !(lib.hasPrefix "." name && builtins.elem name skipDirs)
          # A convention directory is not a subworkspace.
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

  # discoverAll against one subworkspace.
  discoverSubworkspaceOutputs = subworkspaceRoot: conventionOverrides:
    discoverAll subworkspaceRoot conventionOverrides;

  # Each subworkspace with its contents:
  #   { <dir-name> = { path; hasWorkspaceNcl; discovered }; ... }
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

  # In a subworkspace named "foo": packages/default.ncl → "foo", and
  # packages/bar.ncl → "foo-bar".
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

  # The same structure with its output names namespaced:
  #   default   → subworkspaceName
  #   otherName → subworkspaceName-otherName
  namespaceSubworkspaceDiscovered = subworkspaceName: discovered:
    resolveNames {
      workspaceName = subworkspaceName;
      isSubworkspace = true;
    }
    discovered;

  # One tree from root and subworkspaces:
  #   { merged; subworkspaceNames; subworkspaceInfo }
  mergeDiscovered = rootDiscovered: subworkspaceMap: let
    subNames = builtins.attrNames subworkspaceMap;

    namespacedSubs =
      lib.mapAttrs (
        name: info:
          info
          // {
            namespaced = namespaceSubworkspaceDiscovered name info.discovered;
          }
      )
      subworkspaceMap;

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

  # Conflicts between root and subworkspaces, and between subworkspaces.
  checkDiscoveryConflicts = rootDiscovered: subworkspaceMap: let
    subNames = builtins.attrNames subworkspaceMap;

    # { convention.outputName = [source, ...] }
    rootRegistry =
      lib.mapAttrs (
        _convention: outputs:
          lib.mapAttrs (_name: _: ["root"]) outputs
      )
      rootDiscovered;

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
