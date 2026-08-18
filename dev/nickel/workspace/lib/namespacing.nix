# Namespace resolution and conflict detection for nix-workspace subworkspaces
#
# Handles the automatic namespacing of subworkspace outputs and detection
# of naming conflicts. When subworkspaces produce outputs, they are
# automatically prefixed with the subworkspace directory name:
#
#   Root workspace:
#     packages/hello.ncl      → packages.<system>.hello
#
#   Subworkspace "mojo-zed":
#     packages/default.ncl    → packages.<system>.mojo-zed
#     packages/lsp.ncl        → packages.<system>.mojo-zed-lsp
#
# Conflict detection catches:
#   - Two subworkspaces producing the same namespaced output name
#   - A subworkspace output colliding with a root workspace output
#   - Invalid derivation names after namespacing
{lib}: let
  # ── Name resolution ─────────────────────────────────────────────
  #   "default" → subworkspaceName
  #   other     → "${subworkspaceName}-${outputName}"
  namespacedName = subworkspaceName: outputName:
    if outputName == "default"
    then subworkspaceName
    else "${subworkspaceName}-${outputName}";

  # A flat { name = value; } attrset, renamed.
  namespaceOutputs = subworkspaceName: outputs:
    lib.mapAttrs' (
      name: value: {
        name = namespacedName subworkspaceName name;
        inherit value;
      }
    )
    outputs;

  # Every convention directory of a subworkspace, so
  #   { packages = { default = ...; lsp = ...; }; shells = { default = ...; }; }
  # becomes
  #   { packages = { mojo-zed = ...; mojo-zed-lsp = ...; }; shells = { mojo-zed = ...; }; }
  namespaceDiscovered = subworkspaceName: discovered:
    lib.mapAttrs (
      _conventionName: outputs:
        namespaceOutputs subworkspaceName outputs
    )
    discovered;

  # ── Conflict detection ──────────────────────────────────────────

  # Conflicts between root and subworkspaces, and between subworkspaces.
  detectConflicts = rootOutputs: subworkspaceOutputs: let
    # { convention.name = [source, ...] }, where a source is "root" or
    # "subworkspace:mojo-zed".
    registryWithRoot =
      lib.mapAttrs (
        _convention: outputs:
          lib.mapAttrs (
            _name: _value: ["root"]
          )
          outputs
      )
      rootOutputs;

    registryWithAll =
      builtins.foldl' (
        registry: sub: let
          subName = sub.name;
          subOutputs = sub.outputs or {};
        in
          lib.mapAttrs (
            convention: existingNames: let
              newNames = subOutputs.${convention} or {};
              merged =
                existingNames
                // (lib.mapAttrs (
                    name: _value: let
                      existing = existingNames.${name} or [];
                    in
                      existing ++ ["subworkspace:${subName}"]
                  )
                  newNames);
            in
              merged
          )
          registry
      )
      registryWithRoot
      subworkspaceOutputs;

    # More than one source is a conflict.
    conflictsList = lib.concatLists (
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
                    message = "Namespace conflict: '${name}' in '${convention}' is produced by multiple sources: ${builtins.concatStringsSep ", " sources}";
                    hint = "Rename one of the conflicting outputs, or use a different subworkspace directory name.";
                  }
                ]
                else []
            )
            names
          )
      )
      registryWithAll
    );
  in {
    conflicts = conflictsList;
    hasConflicts = conflictsList != [];
  };

  # A valid Nix derivation name is [a-zA-Z_][a-zA-Z0-9_-]*.
  isValidOutputName = name:
    builtins.match "[a-zA-Z_][a-zA-Z0-9_-]*" name != null;

  # Diagnostics for every namespaced name that is not one.
  validateOutputNames = subworkspaceName: outputs:
    lib.concatLists (
      lib.mapAttrsToList (
        name: _value:
          if isValidOutputName name
          then []
          else [
            {
              code = "NW201";
              severity = "error";
              inherit name;
              source = "subworkspace:${subworkspaceName}";
              message = "Invalid output name '${name}' produced by subworkspace '${subworkspaceName}'. Names must match [a-zA-Z_][a-zA-Z0-9_-]*.";
              hint = "Rename the subworkspace directory or the .ncl file to produce a valid name.";
            }
          ]
      )
      outputs
    );

  # ── Merging ─────────────────────────────────────────────────────

  # The entry point for combining outputs: namespace each subworkspace, throw on
  # any conflict, and return the merged { convention = { name = config; }; ... }.
  mergeOutputs = rootOutputs: subworkspaceEntries: let
    namespacedEntries =
      map (
        sub: {
          inherit (sub) name;
          outputs = namespaceDiscovered sub.name (sub.outputs or {});
        }
      )
      subworkspaceEntries;

    conflictResult = detectConflicts rootOutputs namespacedEntries;

    nameValidationErrors = lib.concatLists (
      map (
        sub:
          lib.concatLists (
            lib.mapAttrsToList (
              _convention: outputs:
                validateOutputNames sub.name outputs
            )
            sub.outputs
          )
      )
      namespacedEntries
    );

    allDiagnostics = conflictResult.conflicts ++ nameValidationErrors;

    formatDiagnostic = d:
      "[${d.code}] ${d.message}"
      + (
        if d ? hint
        then "\n  hint: ${d.hint}"
        else ""
      );

    diagnosticMessages =
      builtins.concatStringsSep "\n\n" (map formatDiagnostic allDiagnostics);

    merged =
      builtins.foldl' (
        acc: sub:
          lib.mapAttrs (
            convention: rootNames: let
              subNames = sub.outputs.${convention} or {};
            in
              rootNames // subNames
          )
          acc
      )
      rootOutputs
      namespacedEntries;
  in
    if allDiagnostics != []
    then
      throw ''
        nix-workspace: namespace conflicts detected:

        ${diagnosticMessages}
      ''
    else merged;

  # ── Dependency resolution ───────────────────────────────────────

  # Given a subworkspace's { alias = "subworkspace-name"; ... } and the set of
  # all subworkspace names, diagnose the references that resolve to nothing.
  validateDependencies = subworkspaceName: dependencies: knownSubworkspaces:
    lib.concatLists (
      lib.mapAttrsToList (
        alias: target:
          if builtins.elem target knownSubworkspaces
          then []
          else [
            {
              code = "NW300";
              severity = "error";
              source = "subworkspace:${subworkspaceName}";
              inherit alias target;
              message = "Subworkspace '${subworkspaceName}' declares dependency '${alias}' → '${target}', but no subworkspace named '${target}' exists.";
              hint = let
                suggestions =
                  builtins.filter (
                    name: lib.hasPrefix (builtins.substring 0 3 target) name
                  )
                  knownSubworkspaces;
              in
                if suggestions != []
                then "Did you mean one of: ${builtins.concatStringsSep ", " suggestions}?"
                else "Available subworkspaces: ${builtins.concatStringsSep ", " knownSubworkspaces}";
            }
          ]
      )
      dependencies
    );

  # DFS, which is enough: a workspace dependency graph is tiny.
  detectCycles = dependencyGraph: let
    allNames = builtins.attrNames dependencyGraph;

    # Whether `target` is reachable from `start`.
    isReachable = start: target: visited: let
      neighbors = dependencyGraph.${start} or [];
      unvisited = builtins.filter (n: !builtins.elem n visited) neighbors;
    in
      builtins.elem target neighbors
      || builtins.any (
        neighbor:
          isReachable neighbor target (visited ++ [start])
      )
      unvisited;

    nodesInCycles =
      builtins.filter (
        name: isReachable name name []
      )
      allNames;
  in
    if nodesInCycles == []
    then []
    else [
      {
        code = "NW301";
        severity = "error";
        message = "Circular dependency detected among subworkspaces: ${builtins.concatStringsSep ", " nodesInCycles}";
        hint = "Break the cycle by removing one of the dependency declarations.";
        nodes = nodesInCycles;
      }
    ];

  # { name = ["target1" "target2"]; ... }
  buildDependencyGraph = subworkspaceConfigs:
    lib.mapAttrs (
      _name: config:
        builtins.attrValues (config.dependencies or {})
    )
    subworkspaceConfigs;

  # Every reference resolves, and no cycles.
  validateAllDependencies = subworkspaceConfigs: let
    knownNames = builtins.attrNames subworkspaceConfigs;

    refErrors = lib.concatLists (
      lib.mapAttrsToList (
        name: config:
          validateDependencies name (config.dependencies or {}) knownNames
      )
      subworkspaceConfigs
    );

    graph = buildDependencyGraph subworkspaceConfigs;
    cycleErrors = detectCycles graph;
  in
    refErrors ++ cycleErrors;

  # ── Structured diagnostics ──────────────────────────────────────

  # The structured form the SPEC defines for programmatic consumers:
  #   { diagnostics = [{ code, severity, message, hint, context }]; }
  toStructuredDiagnostics = workspaceName: diagnostics: {
    diagnostics =
      map (
        d:
          {
            inherit (d) code severity message;
            context = {
              workspace = workspaceName;
            };
          }
          // (lib.optionalAttrs (d ? hint) {inherit (d) hint;})
          // (lib.optionalAttrs (d ? name) {field = d.name;})
          // (lib.optionalAttrs (d ? convention) {
            context = {
              workspace = workspaceName;
              output = "${d.convention}.${d.name}";
            };
          })
      )
      diagnostics;
  };
in {
  inherit
    namespacedName
    namespaceOutputs
    namespaceDiscovered
    detectConflicts
    isValidOutputName
    validateOutputNames
    mergeOutputs
    validateDependencies
    detectCycles
    buildDependencyGraph
    validateAllDependencies
    toStructuredDiagnostics
    ;
}
