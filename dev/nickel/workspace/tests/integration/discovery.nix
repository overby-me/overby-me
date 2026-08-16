# Integration tests for nix-workspace subworkspace discovery
#
# These tests exercise the Nix-side discovery module, verifying that
# subworkspace discovery, recursive scanning, and merge operations
# work correctly with the convention directory system.
#
# Run with:
#   nix eval --file tests/integration/discovery.nix
#
# Or via the flake check:
#   nix flake check
#
let
  pkgs = import <nixpkgs> {};
  inherit (pkgs) lib;

  discover = import ../../lib/discover.nix {inherit lib;};

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
    else throw "FAIL: ${name} — expected attribute '${attr}' in ${builtins.toJSON (builtins.attrNames set)}";

  checkNoAttr = name: attr: set:
    if !builtins.hasAttr attr set
    then "PASS: ${name}"
    else throw "FAIL: ${name} — did NOT expect attribute '${attr}' in ${builtins.toJSON (builtins.attrNames set)}";

  assertLength = name: expected: list: let
    actual = builtins.length list;
  in
    if expected == actual
    then "PASS: ${name}"
    else throw "FAIL: ${name} — expected length ${toString expected}, got ${toString actual}";

  # ── discoverSubworkspaces with monorepo example ─────────────────

  monorepoRoot = ../../examples/monorepo;

  discoverSubworkspacesMonorepoTests = let
    subs = discover.discoverSubworkspaces monorepoRoot;
    subNames = builtins.attrNames subs;
  in [
    (assertHasAttr "discoverSubworkspaces monorepo: finds lib-a"
      "lib-a"
      subs)
    (assertHasAttr "discoverSubworkspaces monorepo: finds app-b"
      "app-b"
      subs)
    # packages/ and shells/ directories should NOT appear as subworkspaces
    (checkNoAttr "discoverSubworkspaces monorepo: excludes packages dir"
      "packages"
      subs)
    (check "discoverSubworkspaces monorepo: exactly 2 subworkspaces"
      (builtins.length subNames == 2))
  ];

  # ── discoverSubworkspaces with submodule example ────────────────

  submoduleRoot = ../../examples/submodule;

  discoverSubworkspacesSubmoduleTests = let
    subs = discover.discoverSubworkspaces submoduleRoot;
    subNames = builtins.attrNames subs;
  in [
    (assertHasAttr "discoverSubworkspaces submodule: finds external-tool"
      "external-tool"
      subs)
    (checkNoAttr "discoverSubworkspaces submodule: excludes packages dir"
      "packages"
      subs)
    (check "discoverSubworkspaces submodule: exactly 1 subworkspace"
      (builtins.length subNames == 1))
  ];

  # ── discoverSubworkspaces with minimal example (no subworkspaces) ──

  minimalRoot = ../../examples/minimal;

  discoverSubworkspacesMinimalTests = let
    subs = discover.discoverSubworkspaces minimalRoot;
  in [
    (assertEq "discoverSubworkspaces minimal: no subworkspaces" {} subs)
  ];

  # ── discoverSubworkspaces with nixos example (no subworkspaces) ──

  nixosRoot = ../../examples/nixos;

  discoverSubworkspacesNixosTests = let
    subs = discover.discoverSubworkspaces nixosRoot;
  in [
    (assertEq "discoverSubworkspaces nixos: no subworkspaces" {} subs)
  ];

  # ── discoverAllSubworkspaces ────────────────────────────────────

  discoverAllSubworkspacesMonorepoTests = let
    allSubs = discover.discoverAllSubworkspaces monorepoRoot;
  in [
    (assertHasAttr "discoverAllSubworkspaces: has lib-a" "lib-a" allSubs)
    (assertHasAttr "discoverAllSubworkspaces: has app-b" "app-b" allSubs)

    # Each entry should have path, hasWorkspaceNcl, and discovered
    (check "discoverAllSubworkspaces: lib-a has path"
      (allSubs.lib-a ? path))
    (check "discoverAllSubworkspaces: lib-a hasWorkspaceNcl"
      allSubs.lib-a.hasWorkspaceNcl)
    (check "discoverAllSubworkspaces: lib-a has discovered"
      (allSubs.lib-a ? discovered))

    (check "discoverAllSubworkspaces: app-b has path"
      (allSubs.app-b ? path))
    (check "discoverAllSubworkspaces: app-b hasWorkspaceNcl"
      allSubs.app-b.hasWorkspaceNcl)
    (check "discoverAllSubworkspaces: app-b has discovered"
      (allSubs.app-b ? discovered))

    # lib-a should have packages discovered
    (assertHasAttr "discoverAllSubworkspaces: lib-a has packages convention"
      "packages"
      allSubs.lib-a.discovered)
    (assertHasAttr "discoverAllSubworkspaces: lib-a packages has default"
      "default"
      allSubs.lib-a.discovered.packages)

    # app-b should have packages discovered with default and cli
    (assertHasAttr "discoverAllSubworkspaces: app-b has packages convention"
      "packages"
      allSubs.app-b.discovered)
    (assertHasAttr "discoverAllSubworkspaces: app-b packages has default"
      "default"
      allSubs.app-b.discovered.packages)
    (assertHasAttr "discoverAllSubworkspaces: app-b packages has cli"
      "cli"
      allSubs.app-b.discovered.packages)
  ];

  discoverAllSubworkspacesSubmoduleTests = let
    allSubs = discover.discoverAllSubworkspaces submoduleRoot;
  in [
    (assertHasAttr "discoverAllSubworkspaces submodule: has external-tool"
      "external-tool"
      allSubs)
    (assertHasAttr "discoverAllSubworkspaces submodule: external-tool has packages"
      "packages"
      allSubs.external-tool.discovered)
    (assertHasAttr "discoverAllSubworkspaces submodule: external-tool packages has default"
      "default"
      allSubs.external-tool.discovered.packages)
    (assertHasAttr "discoverAllSubworkspaces submodule: external-tool packages has lib"
      "lib"
      allSubs.external-tool.discovered.packages)
  ];

  # ── resolveNames ────────────────────────────────────────────────

  resolveNamesRootTests = let
    discovered = {
      packages = {
        hello = "/path/to/packages/hello.ncl";
        world = "/path/to/packages/world.ncl";
      };
      shells = {
        default = "/path/to/shells/default.ncl";
      };
    };
    result =
      discover.resolveNames {
        workspaceName = null;
        isSubworkspace = false;
      }
      discovered;
  in [
    (assertHasAttr "resolveNames root: packages has hello"
      "hello"
      result.packages)
    (assertHasAttr "resolveNames root: packages has world"
      "world"
      result.packages)
    (assertHasAttr "resolveNames root: shells has default"
      "default"
      result.shells)
    (assertEq "resolveNames root: hello path preserved"
      "/path/to/packages/hello.ncl"
      result.packages.hello)
  ];

  resolveNamesSubworkspaceTests = let
    discovered = {
      packages = {
        default = "/path/to/packages/default.ncl";
        lsp = "/path/to/packages/lsp.ncl";
      };
      shells = {
        default = "/path/to/shells/default.ncl";
        dev = "/path/to/shells/dev.ncl";
      };
    };
    result =
      discover.resolveNames {
        workspaceName = "mojo-zed";
        isSubworkspace = true;
      }
      discovered;
  in [
    # "default" should become the subworkspace name
    (assertHasAttr "resolveNames sub: packages default → mojo-zed"
      "mojo-zed"
      result.packages)
    (checkNoAttr "resolveNames sub: no raw default in packages"
      "default"
      result.packages)

    # Named outputs get prefixed
    (assertHasAttr "resolveNames sub: packages lsp → mojo-zed-lsp"
      "mojo-zed-lsp"
      result.packages)
    (checkNoAttr "resolveNames sub: no raw lsp in packages"
      "lsp"
      result.packages)

    # Shells follow the same pattern
    (assertHasAttr "resolveNames sub: shells default → mojo-zed"
      "mojo-zed"
      result.shells)
    (assertHasAttr "resolveNames sub: shells dev → mojo-zed-dev"
      "mojo-zed-dev"
      result.shells)

    # Values are preserved
    (assertEq "resolveNames sub: mojo-zed value"
      "/path/to/packages/default.ncl"
      result.packages.mojo-zed)
    (assertEq "resolveNames sub: mojo-zed-lsp value"
      "/path/to/packages/lsp.ncl"
      result.packages.mojo-zed-lsp)
  ];

  # ── namespaceSubworkspaceDiscovered ─────────────────────────────

  namespaceSubworkspaceDiscoveredTests = let
    discovered = {
      packages = {
        default = "/sub/packages/default.ncl";
        cli = "/sub/packages/cli.ncl";
      };
      shells = {
        default = "/sub/shells/default.ncl";
      };
      machines = {};
      modules = {};
    };
    result = discover.namespaceSubworkspaceDiscovered "app-b" discovered;
  in [
    (assertHasAttr "namespaceSubworkspaceDiscovered: app-b in packages"
      "app-b"
      result.packages)
    (assertHasAttr "namespaceSubworkspaceDiscovered: app-b-cli in packages"
      "app-b-cli"
      result.packages)
    (assertHasAttr "namespaceSubworkspaceDiscovered: app-b in shells"
      "app-b"
      result.shells)
    (assertEq "namespaceSubworkspaceDiscovered: machines empty"
      {}
      result.machines)
    (assertEq "namespaceSubworkspaceDiscovered: modules empty"
      {}
      result.modules)
    (checkNoAttr "namespaceSubworkspaceDiscovered: no raw default in packages"
      "default"
      result.packages)
    (checkNoAttr "namespaceSubworkspaceDiscovered: no raw cli in packages"
      "cli"
      result.packages)
  ];

  # ── mergeDiscovered ─────────────────────────────────────────────

  mergeDiscoveredTests = let
    rootDiscovered = {
      packages = {
        shared-lib = "/root/packages/shared-lib.ncl";
      };
      shells = {
        default = "/root/shells/default.ncl";
      };
      machines = {};
      modules = {};
    };

    subworkspaceMap = {
      lib-a = {
        path = "/root/lib-a";
        hasWorkspaceNcl = true;
        discovered = {
          packages = {
            default = "/root/lib-a/packages/default.ncl";
          };
          shells = {};
          machines = {};
          modules = {};
        };
      };
      app-b = {
        path = "/root/app-b";
        hasWorkspaceNcl = true;
        discovered = {
          packages = {
            default = "/root/app-b/packages/default.ncl";
            cli = "/root/app-b/packages/cli.ncl";
          };
          shells = {
            default = "/root/app-b/shells/default.ncl";
          };
          machines = {};
          modules = {};
        };
      };
    };

    result = discover.mergeDiscovered rootDiscovered subworkspaceMap;
  in [
    # Check structure
    (check "mergeDiscovered: has merged" (result ? merged))
    (check "mergeDiscovered: has subworkspaceNames" (result ? subworkspaceNames))
    (check "mergeDiscovered: has subworkspaceInfo" (result ? subworkspaceInfo))

    # Check subworkspace names
    (check "mergeDiscovered: lib-a in names"
      (builtins.elem "lib-a" result.subworkspaceNames))
    (check "mergeDiscovered: app-b in names"
      (builtins.elem "app-b" result.subworkspaceNames))
    (assertLength "mergeDiscovered: 2 subworkspaces" 2 result.subworkspaceNames)

    # Check merged packages
    (assertHasAttr "mergeDiscovered: merged has root shared-lib"
      "shared-lib"
      result.merged.packages)
    (assertHasAttr "mergeDiscovered: merged has lib-a"
      "lib-a"
      result.merged.packages)
    (assertHasAttr "mergeDiscovered: merged has app-b"
      "app-b"
      result.merged.packages)
    (assertHasAttr "mergeDiscovered: merged has app-b-cli"
      "app-b-cli"
      result.merged.packages)
    (checkNoAttr "mergeDiscovered: merged has no raw default in packages"
      "default"
      result.merged.packages)
    (checkNoAttr "mergeDiscovered: merged has no raw cli in packages"
      "cli"
      result.merged.packages)

    # Check merged shells
    (assertHasAttr "mergeDiscovered: merged has root default shell"
      "default"
      result.merged.shells)
    (assertHasAttr "mergeDiscovered: merged has app-b shell"
      "app-b"
      result.merged.shells)

    # Check total counts
    (assertEq "mergeDiscovered: total merged packages"
      4 (builtins.length (builtins.attrNames result.merged.packages)))
    (assertEq "mergeDiscovered: total merged shells"
      2 (builtins.length (builtins.attrNames result.merged.shells)))

    # Check values are correct
    (assertEq "mergeDiscovered: root shared-lib path"
      "/root/packages/shared-lib.ncl"
      result.merged.packages.shared-lib)
    (assertEq "mergeDiscovered: lib-a default path"
      "/root/lib-a/packages/default.ncl"
      result.merged.packages.lib-a)
    (assertEq "mergeDiscovered: app-b default path"
      "/root/app-b/packages/default.ncl"
      result.merged.packages.app-b)
    (assertEq "mergeDiscovered: app-b-cli path"
      "/root/app-b/packages/cli.ncl"
      result.merged.packages.app-b-cli)

    # Check subworkspaceInfo has namespaced entries
    (check "mergeDiscovered: subworkspaceInfo lib-a has namespaced"
      (result.subworkspaceInfo.lib-a ? namespaced))
    (check "mergeDiscovered: subworkspaceInfo app-b has namespaced"
      (result.subworkspaceInfo.app-b ? namespaced))
    (assertHasAttr "mergeDiscovered: lib-a namespaced packages has lib-a"
      "lib-a"
      result.subworkspaceInfo.lib-a.namespaced.packages)
    (assertHasAttr "mergeDiscovered: app-b namespaced packages has app-b"
      "app-b"
      result.subworkspaceInfo.app-b.namespaced.packages)
  ];

  # ── checkDiscoveryConflicts ─────────────────────────────────────

  checkDiscoveryConflictsNoneTests = let
    rootDiscovered = {
      packages = {
        shared-lib = "/root/packages/shared-lib.ncl";
      };
    };
    subworkspaceMap = {
      lib-a = {
        path = "/root/lib-a";
        hasWorkspaceNcl = true;
        discovered = {
          packages = {
            default = "/root/lib-a/packages/default.ncl";
          };
        };
      };
    };
    conflicts = discover.checkDiscoveryConflicts rootDiscovered subworkspaceMap;
  in [
    (assertLength "checkDiscoveryConflicts: no conflicts when distinct" 0 conflicts)
  ];

  checkDiscoveryConflictsRootVsSubTests = let
    rootDiscovered = {
      packages = {
        collider = "/root/packages/collider.ncl";
      };
    };
    # Sub has default → "collider" which clashes with root
    subworkspaceMap = {
      collider = {
        path = "/root/collider";
        hasWorkspaceNcl = true;
        discovered = {
          packages = {
            default = "/root/collider/packages/default.ncl";
          };
        };
      };
    };
    conflicts = discover.checkDiscoveryConflicts rootDiscovered subworkspaceMap;
  in [
    (assertLength "checkDiscoveryConflicts: root vs sub produces 1 conflict" 1 conflicts)
    (assertEq "checkDiscoveryConflicts: conflict code is NW200" "NW200"
      (builtins.head conflicts).code)
    (assertEq "checkDiscoveryConflicts: conflict name is collider" "collider"
      (builtins.head conflicts).name)
    (assertEq "checkDiscoveryConflicts: conflict convention is packages" "packages"
      (builtins.head conflicts).convention)
  ];

  checkDiscoveryConflictsSubVsSubTests = let
    rootDiscovered = {
      packages = {};
    };
    # Two subs where the namespaced names collide:
    # sub-a has output "tool" → "sub-a-tool"
    # sub-b also has output "sub-a-tool" (unusual but possible)
    subworkspaceMap = {
      sub-a = {
        path = "/root/sub-a";
        hasWorkspaceNcl = true;
        discovered = {
          packages = {
            tool = "/root/sub-a/packages/tool.ncl";
          };
        };
      };
      # This sub is named "sub-a-tool" so default → "sub-a-tool", clashing
      sub-a-tool = {
        path = "/root/sub-a-tool";
        hasWorkspaceNcl = true;
        discovered = {
          packages = {
            default = "/root/sub-a-tool/packages/default.ncl";
          };
        };
      };
    };
    conflicts = discover.checkDiscoveryConflicts rootDiscovered subworkspaceMap;
  in [
    (assertLength "checkDiscoveryConflicts: sub vs sub produces 1 conflict" 1 conflicts)
    (assertEq "checkDiscoveryConflicts: sub vs sub conflict name" "sub-a-tool"
      (builtins.head conflicts).name)
  ];

  # ── discoverAll (root-level, verifying convention dirs) ─────────

  discoverAllTests = let
    result = discover.discoverAll monorepoRoot null;
  in [
    (assertHasAttr "discoverAll monorepo: has packages" "packages" result)
    (assertHasAttr "discoverAll monorepo: has shells" "shells" result)
    (assertHasAttr "discoverAll monorepo: packages has shared-lib"
      "shared-lib"
      result.packages)
    # Root discoverAll should NOT see subworkspace packages
    (checkNoAttr "discoverAll monorepo: no lib-a default in root packages"
      "default" (result.packages or {}))
  ];

  # ── Convention override tests ───────────────────────────────────

  conventionOverrideTests = let
    overrides = {
      packages = {
        path = "pkgs";
        auto-discover = true;
      };
      overlays = {
        auto-discover = false;
      };
    };
    applied = discover.applyConventionOverrides discover.defaultConventions overrides;
  in [
    (assertEq "convention override: packages dir changed to pkgs"
      "pkgs"
      applied.packages.dir)
    (check "convention override: packages still auto-discovers"
      (applied.packages.autoDiscover or true))
    (check "convention override: overlays auto-discover disabled"
      (!(applied.overlays.autoDiscover or true)))
    # Non-overridden conventions should keep defaults
    (assertEq "convention override: shells dir unchanged"
      "shells"
      applied.shells.dir)
    (check "convention override: shells still auto-discovers"
      (applied.shells.autoDiscover or true))
  ];

  # ── Edge cases ──────────────────────────────────────────────────

  edgeCaseTests = let
    # discoverSubworkspaces on a path that doesn't exist
    nonExistent = discover.discoverSubworkspaces /tmp/nonexistent-nix-workspace-test;

    # mergeDiscovered with no subworkspaces
    emptyMerge =
      discover.mergeDiscovered
      {packages = {hello = "v";};}
      {};
  in [
    (assertEq "edge case: discoverSubworkspaces on nonexistent returns empty"
      {}
      nonExistent)
    (assertEq "edge case: mergeDiscovered with no subs preserves root"
      {hello = "v";}
      emptyMerge.merged.packages)
    (assertLength "edge case: mergeDiscovered with no subs — 0 subworkspace names"
      0
      emptyMerge.subworkspaceNames)
  ];

  # ── Collect all test results ────────────────────────────────────

  allTests =
    discoverSubworkspacesMonorepoTests
    ++ discoverSubworkspacesSubmoduleTests
    ++ discoverSubworkspacesMinimalTests
    ++ discoverSubworkspacesNixosTests
    ++ discoverAllSubworkspacesMonorepoTests
    ++ discoverAllSubworkspacesSubmoduleTests
    ++ resolveNamesRootTests
    ++ resolveNamesSubworkspaceTests
    ++ namespaceSubworkspaceDiscoveredTests
    ++ mergeDiscoveredTests
    ++ checkDiscoveryConflictsNoneTests
    ++ checkDiscoveryConflictsRootVsSubTests
    ++ checkDiscoveryConflictsSubVsSubTests
    ++ discoverAllTests
    ++ conventionOverrideTests
    ++ edgeCaseTests;

  totalCount = builtins.length allTests;
in
  # Force evaluation of all tests and report
  builtins.deepSeq allTests
  "All ${toString totalCount} discovery integration tests passed."
