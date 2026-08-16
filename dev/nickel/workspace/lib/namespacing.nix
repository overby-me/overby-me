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
#
{lib}: let
  # ── Name resolution ─────────────────────────────────────────────
  # Resolve a single output name within a subworkspace context.
  #
  # Type: String -> String -> String
  #
  # Arguments:
  #   subworkspaceName — The subworkspace directory name (e.g. "mojo-zed")
  #   outputName       — The original output name (e.g. "default", "lsp")
  #
  # Returns:
  #   The namespaced name:
  #     "default" → subworkspaceName
  #     other     → "${subworkspaceName}-${outputName}"
  #
  namespacedName = subworkspaceName: outputName:
    if outputName == "default"
    then subworkspaceName
    else "${subworkspaceName}-${outputName}";

  # Namespace all outputs in a flat { name = value; } attrset for a subworkspace.
  #
  # Type: String -> AttrSet -> AttrSet
  #
  # Arguments:
  #   subworkspaceName — The subworkspace directory name
  #   outputs          — { outputName = value; ... }
  #
  # Returns:
  #   { namespacedName = value; ... }
  #
  namespaceOutputs = subworkspaceName: outputs:
    lib.mapAttrs' (
      name: value: {
        name = namespacedName subworkspaceName name;
        inherit value;
      }
    )
    outputs;

  # Namespace all convention directories for a subworkspace.
  #
  # Type: String -> AttrSet -> AttrSet
  #
  # Given a subworkspace name and a discovered config tree like:
  #   { packages = { default = ...; lsp = ...; }; shells = { default = ...; }; }
  #
  # Returns:
  #   { packages = { mojo-zed = ...; mojo-zed-lsp = ...; }; shells = { mojo-zed = ...; }; }
  #
  namespaceDiscovered = subworkspaceName: discovered:
    lib.mapAttrs (
      _conventionName: outputs:
        namespaceOutputs subworkspaceName outputs
    )
    discovered;

  # ── Conflict detection ──────────────────────────────────────────

  # Check for naming conflicts between the root workspace and all subworkspaces,
  # and between subworkspaces themselves.
  #
  # Type: AttrSet -> [AttrSet] -> AttrSet
  #
  # Arguments:
  #   rootOutputs       — { conventionName = { outputName = ...; }; ... } from root
  #   subworkspaceOutputs — [{ name = "sub-name"; outputs = { conventionName = { ... }; }; }]
  #
  # Returns:
  #   {
  #     conflicts = [ { code, severity, convention, name, sources, message, hint } ];
  #     hasConflicts = Bool;
  #   }
  #
  detectConflicts = rootOutputs: subworkspaceOutputs: let
    # Collect all output registrations: { convention.name = [source1, source2, ...] }
    # where source is a string like "root" or "subworkspace:mojo-zed"
    # Register root outputs
    registryWithRoot =
      lib.mapAttrs (
        _convention: outputs:
          lib.mapAttrs (
            _name: _value: ["root"]
          )
          outputs
      )
      rootOutputs;

    # Register subworkspace outputs into the registry
    registryWithAll =
      builtins.foldl' (
        registry: sub: let
          subName = sub.name;
          subOutputs = sub.outputs or {};
        in
          lib.mapAttrs (
            convention: existingNames: let
              newNames = subOutputs.${convention} or {};
              # For each new name, append the source
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

    # Find all entries with more than one source — those are conflicts
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

  # Validate that a namespaced output name is a valid Nix derivation name.
  #
  # Type: String -> Bool
  #
  # Valid names match: [a-zA-Z_][a-zA-Z0-9_-]*
  isValidOutputName = name:
    builtins.match "[a-zA-Z_][a-zA-Z0-9_-]*" name != null;

  # Validate all namespaced output names and return diagnostics for invalid ones.
  #
  # Type: String -> AttrSet -> [AttrSet]
  #
  # Arguments:
  #   subworkspaceName — The subworkspace name (for error context)
  #   outputs          — { namespacedName = value; ... }
  #
  # Returns: List of diagnostic records for invalid names
  #
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

  # Merge root workspace outputs with namespaced subworkspace outputs.
  #
  # This is the main entry point for combining outputs. It:
  #   1. Namespaces each subworkspace's outputs
  #   2. Checks for conflicts
  #   3. Throws if conflicts are found
  #   4. Returns the merged output tree
  #
  # Type: AttrSet -> [AttrSet] -> AttrSet
  #
  # Arguments:
  #   rootOutputs         — { convention = { name = config; }; ... } from root workspace
  #   subworkspaceEntries — [{ name = "dir-name"; outputs = { convention = { name = config; }; }; }]
  #
  # Returns:
  #   Merged { convention = { name = config; }; ... } with namespaced subworkspace outputs
  #
  mergeOutputs = rootOutputs: subworkspaceEntries: let
    # Namespace each subworkspace's outputs
    namespacedEntries =
      map (
        sub: {
          inherit (sub) name;
          outputs = namespaceDiscovered sub.name (sub.outputs or {});
        }
      )
      subworkspaceEntries;

    # Detect conflicts
    conflictResult = detectConflicts rootOutputs namespacedEntries;

    # Validate all namespaced output names
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

    # Format diagnostics for error output
    formatDiagnostic = d:
      "[${d.code}] ${d.message}"
      + (
        if d ? hint
        then "\n  hint: ${d.hint}"
        else ""
      );

    diagnosticMessages =
      builtins.concatStringsSep "\n\n" (map formatDiagnostic allDiagnostics);

    # Merge all namespaced outputs into the root
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

  # Resolve cross-subworkspace dependencies.
  #
  # Given a dependency map { alias = "subworkspace-name"; ... } from a subworkspace
  # and the set of all subworkspace names, validate that all referenced
  # subworkspaces exist.
  #
  # Type: String -> AttrSet -> [String] -> [AttrSet]
  #
  # Arguments:
  #   subworkspaceName — The subworkspace declaring dependencies
  #   dependencies     — { alias = "target-subworkspace"; ... }
  #   knownSubworkspaces — List of all known subworkspace names
  #
  # Returns: List of diagnostic records for unresolved dependencies
  #
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
                # Simple suggestion: find names that share a prefix
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

  # Detect circular dependencies between subworkspaces.
  #
  # Type: AttrSet -> [AttrSet]
  #
  # Arguments:
  #   dependencyGraph — { subworkspaceName = [targetNames]; ... }
  #
  # Returns: List of diagnostic records for cycles detected
  #
  # Uses a simple DFS-based cycle detection. Since workspace dependency
  # graphs are typically very small, this is sufficient.
  #
  detectCycles = dependencyGraph: let
    allNames = builtins.attrNames dependencyGraph;

    # For each node, do a DFS and check if we revisit it
    # Returns true if `target` is reachable from `start` following edges
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

    # A node is in a cycle if it can reach itself
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

  # Build the full dependency graph from subworkspace configs.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   subworkspaceConfigs — { name = { dependencies = { alias = "target"; }; ... }; ... }
  #
  # Returns:
  #   { name = ["target1" "target2"]; ... }
  #
  buildDependencyGraph = subworkspaceConfigs:
    lib.mapAttrs (
      _name: config:
        builtins.attrValues (config.dependencies or {})
    )
    subworkspaceConfigs;

  # Perform full dependency validation: check all references exist and no cycles.
  #
  # Type: AttrSet -> [AttrSet]
  #
  # Arguments:
  #   subworkspaceConfigs — { name = { dependencies = { ... }; ... }; ... }
  #
  # Returns: List of all dependency-related diagnostics
  #
  validateAllDependencies = subworkspaceConfigs: let
    knownNames = builtins.attrNames subworkspaceConfigs;

    # Validate each subworkspace's dependency references
    refErrors = lib.concatLists (
      lib.mapAttrsToList (
        name: config:
          validateDependencies name (config.dependencies or {}) knownNames
      )
      subworkspaceConfigs
    );

    # Check for cycles
    graph = buildDependencyGraph subworkspaceConfigs;
    cycleErrors = detectCycles graph;
  in
    refErrors ++ cycleErrors;

  # ── Structured diagnostics ──────────────────────────────────────

  # Convert a list of diagnostic records to the structured JSON format
  # described in the SPEC for programmatic consumption.
  #
  # Type: String -> [AttrSet] -> AttrSet
  #
  # Arguments:
  #   workspaceName — The root workspace name (for context)
  #   diagnostics   — List of diagnostic records
  #
  # Returns:
  #   { diagnostics = [{ code, severity, message, hint, context }]; }
  #
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
