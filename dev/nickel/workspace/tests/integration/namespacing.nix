# Integration tests for nix-workspace namespacing logic
#
# These tests exercise the Nix-side namespacing module directly,
# verifying name resolution, conflict detection, dependency validation,
# cycle detection, and output merging.
#
# Run with:
#   nix eval --file tests/integration/namespacing.nix
#
# Or via the flake check:
#   nix flake check
#
let
  pkgs = import <nixpkgs> {};
  inherit (pkgs) lib;

  namespacingLib = import ../../lib/namespacing.nix {inherit lib;};

  # ── Test helpers ────────────────────────────────────────────────

  check = name: condition:
    if condition
    then "PASS: ${name}"
    else throw "FAIL: ${name}";

  assertEq = name: expected: actual:
    if expected == actual
    then "PASS: ${name}"
    else throw "FAIL: ${name} — expected ${builtins.toJSON expected}, got ${builtins.toJSON actual}";

  assertHasAttr = name: attr: set:
    if builtins.hasAttr attr set
    then "PASS: ${name}"
    else throw "FAIL: ${name} — expected attribute '${attr}' in ${builtins.toJSON set}";

  assertLength = name: expected: list: let
    actual = builtins.length list;
  in
    if expected == actual
    then "PASS: ${name}"
    else throw "FAIL: ${name} — expected length ${toString expected}, got ${toString actual}";

  # ── namespacedName tests ────────────────────────────────────────

  namespacedNameTests = [
    (assertEq "namespacedName: default -> subworkspace name"
      "mojo-zed"
      (namespacingLib.namespacedName "mojo-zed" "default"))

    (assertEq "namespacedName: named output -> prefixed"
      "mojo-zed-lsp"
      (namespacingLib.namespacedName "mojo-zed" "lsp"))

    (assertEq "namespacedName: named output -> prefixed with hyphen"
      "app-b-my-tool"
      (namespacingLib.namespacedName "app-b" "my-tool"))

    (assertEq "namespacedName: single char subworkspace"
      "a"
      (namespacingLib.namespacedName "a" "default"))

    (assertEq "namespacedName: single char subworkspace, named output"
      "a-foo"
      (namespacingLib.namespacedName "a" "foo"))

    (assertEq "namespacedName: underscore subworkspace"
      "_internal"
      (namespacingLib.namespacedName "_internal" "default"))

    (assertEq "namespacedName: underscore subworkspace, named"
      "_internal-utils"
      (namespacingLib.namespacedName "_internal" "utils"))
  ];

  # ── namespaceOutputs tests ──────────────────────────────────────

  namespaceOutputsTests = let
    outputs = {
      default = "default-value";
      lsp = "lsp-value";
      cli = "cli-value";
    };
    result = namespacingLib.namespaceOutputs "mojo-zed" outputs;
  in [
    (assertHasAttr "namespaceOutputs: has mojo-zed from default" "mojo-zed" result)
    (assertHasAttr "namespaceOutputs: has mojo-zed-lsp" "mojo-zed-lsp" result)
    (assertHasAttr "namespaceOutputs: has mojo-zed-cli" "mojo-zed-cli" result)
    (assertEq "namespaceOutputs: mojo-zed value" "default-value" result.mojo-zed)
    (assertEq "namespaceOutputs: mojo-zed-lsp value" "lsp-value" result.mojo-zed-lsp)
    (assertEq "namespaceOutputs: mojo-zed-cli value" "cli-value" result.mojo-zed-cli)
    (check "namespaceOutputs: no default key" (!builtins.hasAttr "default" result))
  ];

  # ── namespaceDiscovered tests ───────────────────────────────────

  namespaceDiscoveredTests = let
    discovered = {
      packages = {
        default = "/path/to/default.ncl";
        lsp = "/path/to/lsp.ncl";
      };
      shells = {
        default = "/path/to/shells/default.ncl";
      };
      machines = {};
      modules = {
        networking = "/path/to/modules/networking.ncl";
      };
    };
    result = namespacingLib.namespaceDiscovered "infra" discovered;
  in [
    (assertHasAttr "namespaceDiscovered: packages has infra" "infra" result.packages)
    (assertHasAttr "namespaceDiscovered: packages has infra-lsp" "infra-lsp" result.packages)
    (assertHasAttr "namespaceDiscovered: shells has infra" "infra" result.shells)
    (assertEq "namespaceDiscovered: machines empty" {} result.machines)
    (assertHasAttr "namespaceDiscovered: modules has infra-networking" "infra-networking" result.modules)
    (check "namespaceDiscovered: no raw default in packages" (!builtins.hasAttr "default" result.packages))
    (check "namespaceDiscovered: no raw networking in modules" (!builtins.hasAttr "networking" result.modules))
  ];

  # ── isValidOutputName tests ─────────────────────────────────────

  isValidOutputNameTests = [
    (check "isValidOutputName: simple" (namespacingLib.isValidOutputName "hello"))
    (check "isValidOutputName: hyphenated" (namespacingLib.isValidOutputName "my-tool"))
    (check "isValidOutputName: underscored" (namespacingLib.isValidOutputName "my_tool"))
    (check "isValidOutputName: starts with underscore" (namespacingLib.isValidOutputName "_private"))
    (check "isValidOutputName: mixed" (namespacingLib.isValidOutputName "Foo_Bar-123"))
    (check "isValidOutputName: rejects empty" (!namespacingLib.isValidOutputName ""))
    (check "isValidOutputName: rejects starts with number" (!namespacingLib.isValidOutputName "1bad"))
    (check "isValidOutputName: rejects starts with hyphen" (!namespacingLib.isValidOutputName "-bad"))
    (check "isValidOutputName: rejects spaces" (!namespacingLib.isValidOutputName "has space"))
    (check "isValidOutputName: rejects dots" (!namespacingLib.isValidOutputName "has.dot"))
    (check "isValidOutputName: rejects slash" (!namespacingLib.isValidOutputName "has/slash"))
  ];

  # ── detectConflicts tests ───────────────────────────────────────

  detectConflictsNoConflictTests = let
    rootOutputs = {
      packages = {
        shared-lib = "root-shared-lib";
      };
      shells = {
        default = "root-default-shell";
      };
    };
    subEntries = [
      {
        name = "lib-a";
        outputs = {
          packages = {
            lib-a = "lib-a-default";
          };
          shells = {
            lib-a = "lib-a-shell";
          };
        };
      }
      {
        name = "app-b";
        outputs = {
          packages = {
            app-b = "app-b-default";
            app-b-cli = "app-b-cli";
          };
          shells = {};
        };
      }
    ];
    result = namespacingLib.detectConflicts rootOutputs subEntries;
  in [
    (check "detectConflicts: no conflicts when names are distinct"
      (!result.hasConflicts))
    (assertLength "detectConflicts: no conflict entries" 0 result.conflicts)
  ];

  detectConflictsRootVsSubTests = let
    rootOutputs = {
      packages = {
        my-tool = "root-my-tool";
      };
    };
    subEntries = [
      {
        name = "sub-a";
        outputs = {
          packages = {
            my-tool = "sub-a-my-tool"; # conflicts with root
          };
        };
      }
    ];
    result = namespacingLib.detectConflicts rootOutputs subEntries;
  in [
    (check "detectConflicts: root vs sub conflict detected"
      result.hasConflicts)
    (assertLength "detectConflicts: one root vs sub conflict" 1 result.conflicts)
    (assertEq "detectConflicts: root vs sub conflict code" "NW200"
      (builtins.head result.conflicts).code)
    (assertEq "detectConflicts: root vs sub conflict name" "my-tool"
      (builtins.head result.conflicts).name)
  ];

  detectConflictsSubVsSubTests = let
    rootOutputs = {
      packages = {};
    };
    # Two subworkspaces producing the same namespaced output name.
    # This can happen if e.g. sub-a has output "common" → "sub-a-common"
    # and sub-b also has output "sub-a-common" (unlikely but possible).
    subEntries = [
      {
        name = "sub-a";
        outputs = {
          packages = {
            collider = "sub-a-collider";
          };
        };
      }
      {
        name = "sub-b";
        outputs = {
          packages = {
            collider = "sub-b-collider"; # same name as sub-a
          };
        };
      }
    ];
    result = namespacingLib.detectConflicts rootOutputs subEntries;
  in [
    (check "detectConflicts: sub vs sub conflict detected"
      result.hasConflicts)
    (assertLength "detectConflicts: one sub vs sub conflict" 1 result.conflicts)
    (assertEq "detectConflicts: sub vs sub conflict convention" "packages"
      (builtins.head result.conflicts).convention)
  ];

  detectConflictsMultiConventionTests = let
    rootOutputs = {
      packages = {
        collider = "root-pkg";
      };
      shells = {
        collider = "root-shell";
      };
    };
    subEntries = [
      {
        name = "sub-a";
        outputs = {
          packages = {
            collider = "sub-a-pkg";
          };
          shells = {};
        };
      }
    ];
    result = namespacingLib.detectConflicts rootOutputs subEntries;
  in [
    (check "detectConflicts: multi-convention only conflicts in affected convention"
      result.hasConflicts)
    # Only packages.collider conflicts, not shells.collider
    (assertLength "detectConflicts: one conflict (packages only)" 1 result.conflicts)
    (assertEq "detectConflicts: conflict is in packages" "packages"
      (builtins.head result.conflicts).convention)
  ];

  # ── validateOutputNames tests ───────────────────────────────────

  validateOutputNamesTests = let
    validOutputs = {
      my-tool = "v1";
      my_lib = "v2";
      MixedCase = "v3";
    };
    validResult = namespacingLib.validateOutputNames "sub" validOutputs;

    # Simulate an output name that somehow ended up invalid
    # (e.g. from a directory with a dot in the name)
    invalidOutputs = {
      "has.dot" = "v1";
    };
    invalidResult = namespacingLib.validateOutputNames "sub" invalidOutputs;
  in [
    (assertLength "validateOutputNames: valid names produce no diagnostics" 0 validResult)
    (assertLength "validateOutputNames: invalid name produces diagnostic" 1 invalidResult)
    (assertEq "validateOutputNames: invalid name diagnostic code" "NW201"
      (builtins.head invalidResult).code)
  ];

  # ── mergeOutputs tests ──────────────────────────────────────────

  mergeOutputsNoSubsTests = let
    rootOutputs = {
      packages = {
        hello = "root-hello";
        world = "root-world";
      };
      shells = {
        default = "root-default-shell";
      };
    };
    result = namespacingLib.mergeOutputs rootOutputs [];
  in [
    (assertEq "mergeOutputs: no subs returns root packages"
      rootOutputs.packages
      result.packages)
    (assertEq "mergeOutputs: no subs returns root shells"
      rootOutputs.shells
      result.shells)
  ];

  mergeOutputsWithSubsTests = let
    rootOutputs = {
      packages = {
        shared-lib = "root-shared-lib";
      };
      shells = {
        default = "root-default-shell";
      };
    };
    subEntries = [
      {
        name = "lib-a";
        outputs = {
          packages = {
            default = "lib-a-default-pkg";
            utils = "lib-a-utils-pkg";
          };
          shells = {
            default = "lib-a-default-shell";
          };
        };
      }
      {
        name = "app-b";
        outputs = {
          packages = {
            default = "app-b-default-pkg";
            cli = "app-b-cli-pkg";
          };
          shells = {};
        };
      }
    ];
    result = namespacingLib.mergeOutputs rootOutputs subEntries;
  in [
    # Root outputs preserved
    (assertHasAttr "mergeOutputs: root shared-lib preserved" "shared-lib" result.packages)
    (assertHasAttr "mergeOutputs: root default shell preserved" "default" result.shells)

    # lib-a outputs namespaced
    (assertHasAttr "mergeOutputs: lib-a default → lib-a" "lib-a" result.packages)
    (assertHasAttr "mergeOutputs: lib-a utils → lib-a-utils" "lib-a-utils" result.packages)
    (assertHasAttr "mergeOutputs: lib-a shell → lib-a" "lib-a" result.shells)

    # app-b outputs namespaced
    (assertHasAttr "mergeOutputs: app-b default → app-b" "app-b" result.packages)
    (assertHasAttr "mergeOutputs: app-b cli → app-b-cli" "app-b-cli" result.packages)

    # Values correct
    (assertEq "mergeOutputs: lib-a value" "lib-a-default-pkg" result.packages.lib-a)
    (assertEq "mergeOutputs: app-b-cli value" "app-b-cli-pkg" result.packages.app-b-cli)
    (assertEq "mergeOutputs: root shared-lib value" "root-shared-lib" result.packages.shared-lib)

    # No raw "default" keys from subs
    (check "mergeOutputs: no raw default in packages from subs"
      (result.packages.default or null == null))

    # Total count: shared-lib + lib-a + lib-a-utils + app-b + app-b-cli = 5
    (assertEq "mergeOutputs: total packages count" 5
      (builtins.length (builtins.attrNames result.packages)))

    # Shells: default (root) + lib-a = 2
    (assertEq "mergeOutputs: total shells count" 2
      (builtins.length (builtins.attrNames result.shells)))
  ];

  # ── validateDependencies tests ──────────────────────────────────

  validateDependenciesValidTests = let
    result =
      namespacingLib.validateDependencies
      "app-b"
      {
        core = "lib-a";
        utils = "common";
      }
      ["lib-a" "common" "app-b"];
  in [
    (assertLength "validateDependencies: all deps exist — no errors" 0 result)
  ];

  validateDependenciesMissingTests = let
    result =
      namespacingLib.validateDependencies
      "app-b"
      {
        core = "lib-a";
        missing = "nonexistent";
      }
      ["lib-a" "app-b"];
  in [
    (assertLength "validateDependencies: one missing dep" 1 result)
    (assertEq "validateDependencies: missing dep code" "NW300"
      (builtins.head result).code)
    (assertEq "validateDependencies: missing dep target" "nonexistent"
      (builtins.head result).target)
  ];

  validateDependenciesAllMissingTests = let
    result =
      namespacingLib.validateDependencies
      "lonely"
      {
        a = "nope";
        b = "also-nope";
      }
      ["lonely"];
  in [
    (assertLength "validateDependencies: two missing deps" 2 result)
  ];

  # ── detectCycles tests ──────────────────────────────────────────

  detectCyclesNoneTests = let
    graph = {
      app-b = ["lib-a"];
      lib-a = [];
    };
    result = namespacingLib.detectCycles graph;
  in [
    (assertLength "detectCycles: no cycle in linear DAG" 0 result)
  ];

  detectCyclesDirectTests = let
    graph = {
      a = ["b"];
      b = ["a"];
    };
    result = namespacingLib.detectCycles graph;
  in [
    (assertLength "detectCycles: direct cycle detected" 1 result)
    (assertEq "detectCycles: direct cycle code" "NW301"
      (builtins.head result).code)
  ];

  detectCyclesSelfTests = let
    graph = {
      a = ["a"];
    };
    result = namespacingLib.detectCycles graph;
  in [
    (assertLength "detectCycles: self-cycle detected" 1 result)
  ];

  detectCyclesTransitiveTests = let
    graph = {
      a = ["b"];
      b = ["c"];
      c = ["a"];
      d = []; # not in cycle
    };
    result = namespacingLib.detectCycles graph;
  in [
    (assertLength "detectCycles: transitive cycle detected" 1 result)
    # Nodes a, b, c should be in the cycle
    (check "detectCycles: transitive cycle includes a"
      (builtins.elem "a" (builtins.head result).nodes))
    (check "detectCycles: transitive cycle includes b"
      (builtins.elem "b" (builtins.head result).nodes))
    (check "detectCycles: transitive cycle includes c"
      (builtins.elem "c" (builtins.head result).nodes))
    (check "detectCycles: transitive cycle excludes d"
      (!builtins.elem "d" (builtins.head result).nodes))
  ];

  detectCyclesNoCycleChainTests = let
    graph = {
      a = ["b"];
      b = ["c"];
      c = ["d"];
      d = [];
    };
    result = namespacingLib.detectCycles graph;
  in [
    (assertLength "detectCycles: linear chain has no cycle" 0 result)
  ];

  # ── buildDependencyGraph tests ──────────────────────────────────

  buildDependencyGraphTests = let
    configs = {
      app-b = {
        dependencies = {
          core = "lib-a";
          utils = "common";
        };
      };
      lib-a = {};
      common = {
        dependencies = {base = "lib-a";};
      };
    };
    graph = namespacingLib.buildDependencyGraph configs;
  in [
    (assertEq "buildDependencyGraph: app-b deps"
      ["common" "lib-a"]
      (lib.sort builtins.lessThan graph.app-b))
    (assertEq "buildDependencyGraph: lib-a deps (empty)" [] graph.lib-a)
    (assertEq "buildDependencyGraph: common deps" ["lib-a"] graph.common)
  ];

  # ── validateAllDependencies tests ───────────────────────────────

  validateAllDependenciesValidTests = let
    configs = {
      app-b = {
        dependencies = {core = "lib-a";};
      };
      lib-a = {};
    };
    result = namespacingLib.validateAllDependencies configs;
  in [
    (assertLength "validateAllDependencies: valid deps — no errors" 0 result)
  ];

  validateAllDependenciesMissingTests = let
    configs = {
      app-b = {
        dependencies = {core = "nonexistent";};
      };
    };
    result = namespacingLib.validateAllDependencies configs;
  in [
    (assertLength "validateAllDependencies: missing dep produces error" 1 result)
    (assertEq "validateAllDependencies: error code" "NW300"
      (builtins.head result).code)
  ];

  validateAllDependenciesCycleTests = let
    configs = {
      a = {dependencies = {x = "b";};};
      b = {dependencies = {y = "a";};};
    };
    result = namespacingLib.validateAllDependencies configs;
  in [
    (check "validateAllDependencies: cycle produces error"
      (builtins.length result > 0))
    (check "validateAllDependencies: has NW301 cycle error"
      (builtins.any (d: d.code == "NW301") result))
  ];

  # ── toStructuredDiagnostics tests ───────────────────────────────

  toStructuredDiagnosticsTests = let
    diagnostics = [
      {
        code = "NW200";
        severity = "error";
        convention = "packages";
        name = "collider";
        message = "Namespace conflict";
        hint = "Rename something";
        sources = ["root" "subworkspace:sub-a"];
      }
      {
        code = "NW300";
        severity = "error";
        message = "Missing dep";
        source = "subworkspace:sub-b";
        target = "nonexistent";
        alias = "bad";
      }
    ];
    result = namespacingLib.toStructuredDiagnostics "my-workspace" diagnostics;
  in [
    (assertLength "toStructuredDiagnostics: two diagnostics" 2 result.diagnostics)
    (assertEq "toStructuredDiagnostics: first code" "NW200"
      (builtins.head result.diagnostics).code)
    (assertEq "toStructuredDiagnostics: first has hint" "Rename something"
      (builtins.head result.diagnostics).hint)
    (assertEq "toStructuredDiagnostics: first has field" "collider"
      (builtins.head result.diagnostics).field)
    (assertEq "toStructuredDiagnostics: first context output" "packages.collider"
      (builtins.head result.diagnostics).context.output)
    (assertEq "toStructuredDiagnostics: second code" "NW300"
      (builtins.elemAt result.diagnostics 1).code)
    (assertEq "toStructuredDiagnostics: second context workspace" "my-workspace"
      (builtins.elemAt result.diagnostics 1).context.workspace)
  ];

  # ── Edge case: empty everything ─────────────────────────────────

  edgeCaseTests = let
    emptyMerge = namespacingLib.mergeOutputs {
      packages = {};
      shells = {};
    } [];
    emptyConflicts = namespacingLib.detectConflicts {packages = {};} [];
    emptyDeps = namespacingLib.validateAllDependencies {};
    emptyGraph = namespacingLib.detectCycles {};
  in [
    (assertEq "edge case: merge empty root + no subs" {
        packages = {};
        shells = {};
      }
      emptyMerge)
    (check "edge case: no conflicts with empty" (!emptyConflicts.hasConflicts))
    (assertLength "edge case: no deps to validate" 0 emptyDeps)
    (assertLength "edge case: no cycles in empty graph" 0 emptyGraph)
  ];

  # ── Collect all test results ────────────────────────────────────

  allTests =
    namespacedNameTests
    ++ namespaceOutputsTests
    ++ namespaceDiscoveredTests
    ++ isValidOutputNameTests
    ++ detectConflictsNoConflictTests
    ++ detectConflictsRootVsSubTests
    ++ detectConflictsSubVsSubTests
    ++ detectConflictsMultiConventionTests
    ++ validateOutputNamesTests
    ++ mergeOutputsNoSubsTests
    ++ mergeOutputsWithSubsTests
    ++ validateDependenciesValidTests
    ++ validateDependenciesMissingTests
    ++ validateDependenciesAllMissingTests
    ++ detectCyclesNoneTests
    ++ detectCyclesDirectTests
    ++ detectCyclesSelfTests
    ++ detectCyclesTransitiveTests
    ++ detectCyclesNoCycleChainTests
    ++ buildDependencyGraphTests
    ++ validateAllDependenciesValidTests
    ++ validateAllDependenciesMissingTests
    ++ validateAllDependenciesCycleTests
    ++ toStructuredDiagnosticsTests
    ++ edgeCaseTests;

  totalCount = builtins.length allTests;
in
  # Force evaluation of all tests and report
  builtins.deepSeq allTests
  "All ${toString totalCount} namespacing integration tests passed."
