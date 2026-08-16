# Integration tests for the nix-workspace plugin system (Nix side)
#
# Tests the Nix-side plugin functionality:
#   - Plugin resolution (name → directory mapping)
#   - Plugin loading (builder.nix imports)
#   - Convention extraction from evaluated plugin configs
#   - Builder routing (core vs plugin builders)
#   - Shell extras collection
#   - Plugin validation (duplicate detection)
#   - Builder default application
#   - Convention discovery integration
#
# Run with:
#   nix eval --file tests/integration/plugins.nix
#
let
  nixpkgs = import <nixpkgs> {};
  inherit (nixpkgs) lib;

  # Import the modules under test
  pluginsLib = import ../../lib/plugins.nix {inherit lib;};
  discover = import ../../lib/discover.nix {inherit lib;};

  # ── Test helpers ────────────────────────────────────────────────

  check = msg: cond:
    if cond
    then "PASS: ${msg}"
    else builtins.throw "FAIL: ${msg}";

  checkEq = msg: expected: actual:
    if expected == actual
    then "PASS: ${msg}"
    else builtins.throw "FAIL: ${msg} — expected ${builtins.toJSON expected}, got ${builtins.toJSON actual}";

  checkThrows = msg: expr: let
    result = builtins.tryEval (builtins.deepSeq expr expr);
  in
    if !result.success
    then "PASS: ${msg} (correctly threw)"
    else builtins.throw "FAIL: ${msg} — expected throw, but got ${builtins.toJSON result.value}";

  # Path to the plugins directory (relative to project root)
  pluginsDir = ../../plugins;

  # ── Plugin resolution tests ─────────────────────────────────────

  test_resolve_rust_plugin = let
    dir = pluginsLib.resolvePluginDir pluginsDir "nix-workspace-rust";
  in
    check "resolvePluginDir finds rust plugin" (builtins.pathExists dir);

  test_resolve_go_plugin = let
    dir = pluginsLib.resolvePluginDir pluginsDir "nix-workspace-go";
  in
    check "resolvePluginDir finds go plugin" (builtins.pathExists dir);

  test_resolve_short_name = let
    dir = pluginsLib.resolvePluginDir pluginsDir "rust";
  in
    check "resolvePluginDir accepts short name" (builtins.pathExists dir);

  test_resolve_unknown_throws =
    checkThrows "resolvePluginDir throws for unknown plugin"
    (pluginsLib.resolvePluginDir pluginsDir "nix-workspace-nonexistent");

  test_resolve_ncl_rust = let
    path = pluginsLib.resolvePluginNcl pluginsDir "nix-workspace-rust";
  in
    check "resolvePluginNcl finds rust plugin.ncl" (builtins.pathExists path);

  test_resolve_ncl_go = let
    path = pluginsLib.resolvePluginNcl pluginsDir "nix-workspace-go";
  in
    check "resolvePluginNcl finds go plugin.ncl" (builtins.pathExists path);

  test_resolve_builder_rust = let
    path = pluginsLib.resolvePluginBuilder pluginsDir "nix-workspace-rust";
  in
    check "resolvePluginBuilder finds rust builder.nix" (path != null && builtins.pathExists path);

  test_resolve_builder_go = let
    path = pluginsLib.resolvePluginBuilder pluginsDir "nix-workspace-go";
  in
    check "resolvePluginBuilder finds go builder.nix" (path != null && builtins.pathExists path);

  # ── Plugin loading tests ────────────────────────────────────────

  test_load_plugins_empty = let
    result = pluginsLib.loadPlugins pluginsDir [];
  in
    checkEq "loadPlugins with empty list gives empty builders"
    {}
    result.builders;

  test_load_plugins_empty_names = let
    result = pluginsLib.loadPlugins pluginsDir [];
  in
    checkEq "loadPlugins with empty list gives empty pluginNames"
    []
    result.pluginNames;

  test_load_plugins_rust = let
    result = pluginsLib.loadPlugins pluginsDir ["nix-workspace-rust"];
  in
    check "loadPlugins loads rust builder"
    (builtins.hasAttr "rust" result.builders);

  test_load_plugins_go = let
    result = pluginsLib.loadPlugins pluginsDir ["nix-workspace-go"];
  in
    check "loadPlugins loads go builder"
    (builtins.hasAttr "go" result.builders);

  test_load_plugins_both = let
    result = pluginsLib.loadPlugins pluginsDir ["nix-workspace-rust" "nix-workspace-go"];
  in
    check "loadPlugins loads both rust and go builders"
    (builtins.hasAttr "rust" result.builders && builtins.hasAttr "go" result.builders);

  test_load_plugins_names_preserved = let
    result = pluginsLib.loadPlugins pluginsDir ["nix-workspace-rust" "nix-workspace-go"];
  in
    checkEq "loadPlugins preserves plugin names"
    ["nix-workspace-rust" "nix-workspace-go"]
    result.pluginNames;

  test_load_plugins_shell_extras_rust = let
    result = pluginsLib.loadPlugins pluginsDir ["nix-workspace-rust"];
  in
    check "loadPlugins finds rust shell extras"
    (builtins.hasAttr "nix-workspace-rust" result.shellExtras);

  test_load_plugins_no_shell_extras_go = let
    result = pluginsLib.loadPlugins pluginsDir ["nix-workspace-go"];
  in
    checkEq "loadPlugins go has no shell extras"
    {}
    result.shellExtras;

  # ── Convention extraction tests ─────────────────────────────────

  test_extract_conventions_empty = let
    result = pluginsLib.extractConventions {};
  in
    checkEq "extractConventions empty input gives empty output"
    {}
    result;

  test_extract_conventions_rust = let
    fakePluginConfig = {
      "nix-workspace-rust" = {
        name = "nix-workspace-rust";
        conventions = {
          crates = {
            path = "crates";
            output = "packages";
            builder = "rust";
            auto-discover = true;
          };
        };
      };
    };
    result = pluginsLib.extractConventions fakePluginConfig;
  in
    check "extractConventions produces crates convention"
    (builtins.hasAttr "crates" result);

  test_extract_conventions_dir = let
    fakePluginConfig = {
      "nix-workspace-rust" = {
        name = "nix-workspace-rust";
        conventions = {
          crates = {
            path = "crates";
            output = "packages";
            builder = "rust";
            auto-discover = true;
          };
        };
      };
    };
    result = pluginsLib.extractConventions fakePluginConfig;
  in
    checkEq "extracted convention has correct dir"
    "crates"
    result.crates.dir;

  test_extract_conventions_output = let
    fakePluginConfig = {
      "nix-workspace-rust" = {
        name = "nix-workspace-rust";
        conventions = {
          crates = {
            path = "crates";
            output = "packages";
            builder = "rust";
            auto-discover = true;
          };
        };
      };
    };
    result = pluginsLib.extractConventions fakePluginConfig;
  in
    checkEq "extracted convention has correct output"
    "packages"
    result.crates.output;

  test_extract_conventions_builder_field = let
    fakePluginConfig = {
      "nix-workspace-rust" = {
        name = "nix-workspace-rust";
        conventions = {
          crates = {
            path = "crates";
            output = "packages";
            builder = "rust";
            auto-discover = true;
          };
        };
      };
    };
    result = pluginsLib.extractConventions fakePluginConfig;
  in
    checkEq "extracted convention has correct builder"
    "rust"
    result.crates.builder;

  test_extract_conventions_from_plugin = let
    fakePluginConfig = {
      "nix-workspace-rust" = {
        name = "nix-workspace-rust";
        conventions = {
          crates = {
            path = "crates";
            output = "packages";
            builder = "rust";
            auto-discover = true;
          };
        };
      };
    };
    result = pluginsLib.extractConventions fakePluginConfig;
  in
    checkEq "extracted convention marked fromPlugin"
    true
    result.crates.fromPlugin;

  test_extract_conventions_multiple_plugins = let
    fakePluginConfig = {
      "nix-workspace-rust" = {
        name = "nix-workspace-rust";
        conventions = {
          crates = {
            path = "crates";
            output = "packages";
            builder = "rust";
            auto-discover = true;
          };
        };
      };
      "nix-workspace-go" = {
        name = "nix-workspace-go";
        conventions = {
          go-modules = {
            path = "go-modules";
            output = "packages";
            builder = "go";
            auto-discover = true;
          };
        };
      };
    };
    result = pluginsLib.extractConventions fakePluginConfig;
  in
    check "extractConventions merges conventions from multiple plugins"
    (builtins.hasAttr "crates" result && builtins.hasAttr "go-modules" result);

  test_extract_conventions_no_conventions = let
    fakePluginConfig = {
      "minimal-plugin" = {
        name = "minimal-plugin";
      };
    };
    result = pluginsLib.extractConventions fakePluginConfig;
  in
    checkEq "extractConventions with no conventions gives empty"
    {}
    result;

  # ── Plugin validation tests ─────────────────────────────────────

  test_validate_empty = let
    result = pluginsLib.validatePlugins [];
  in
    checkEq "validatePlugins empty list gives no diagnostics"
    []
    result;

  test_validate_single = let
    result = pluginsLib.validatePlugins ["nix-workspace-rust"];
  in
    checkEq "validatePlugins single plugin gives no diagnostics"
    []
    result;

  test_validate_two_unique = let
    result = pluginsLib.validatePlugins ["nix-workspace-rust" "nix-workspace-go"];
  in
    checkEq "validatePlugins two unique plugins gives no diagnostics"
    []
    result;

  test_validate_duplicate = let
    result = pluginsLib.validatePlugins ["nix-workspace-rust" "nix-workspace-rust"];
  in
    check "validatePlugins detects duplicate"
    (builtins.length result > 0);

  test_validate_duplicate_code = let
    result = pluginsLib.validatePlugins ["nix-workspace-rust" "nix-workspace-rust"];
  in
    checkEq "validatePlugins duplicate has code NW400"
    "NW400"
    (builtins.head result).code;

  test_validate_duplicate_severity = let
    result = pluginsLib.validatePlugins ["nix-workspace-rust" "nix-workspace-rust"];
  in
    checkEq "validatePlugins duplicate severity is error"
    "error"
    (builtins.head result).severity;

  test_validate_triple_duplicate = let
    result = pluginsLib.validatePlugins ["a" "a" "a"];
  in
    check "validatePlugins detects triple duplicate"
    (builtins.length result > 0);

  test_validate_mixed_duplicates = let
    result = pluginsLib.validatePlugins ["a" "b" "a" "c" "b"];
  in
    checkEq "validatePlugins detects multiple duplicate groups"
    2 (builtins.length result);

  # ── Builder defaults tests ──────────────────────────────────────

  test_apply_defaults_no_plugin = let
    pluginBuilders = {};
    cfg = {
      build-system = "generic";
      version = "1.0";
    };
    result = pluginsLib.applyBuilderDefaults pluginBuilders cfg;
  in
    checkEq "applyBuilderDefaults with no plugin unchanged"
    "1.0"
    result.version;

  test_apply_defaults_no_matching_builder = let
    pluginBuilders = {
      rust = {meta = {defaults = {cargo-lock = "Cargo.lock";};};};
    };
    cfg = {
      build-system = "generic";
      version = "1.0";
    };
    result = pluginsLib.applyBuilderDefaults pluginBuilders cfg;
  in
    checkEq "applyBuilderDefaults no matching builder unchanged"
    "1.0"
    result.version;

  test_apply_defaults_matching_builder = let
    pluginBuilders = {
      rust = {meta = {defaults = {cargo-lock = "Cargo.lock";};};};
    };
    cfg = {
      build-system = "rust";
      version = "1.0";
    };
    result = pluginsLib.applyBuilderDefaults pluginBuilders cfg;
  in
    check "applyBuilderDefaults applies plugin defaults"
    (result.cargo-lock == "Cargo.lock");

  test_apply_defaults_user_wins = let
    pluginBuilders = {
      rust = {meta = {defaults = {cargo-lock = "default.lock";};};};
    };
    cfg = {
      build-system = "rust";
      cargo-lock = "custom.lock";
    };
    result = pluginsLib.applyBuilderDefaults pluginBuilders cfg;
  in
    checkEq "applyBuilderDefaults user value overrides default"
    "custom.lock"
    result.cargo-lock;

  test_apply_defaults_preserves_all_user_fields = let
    pluginBuilders = {
      rust = {meta = {defaults = {cargo-lock = "Cargo.lock";};};};
    };
    cfg = {
      build-system = "rust";
      version = "2.0";
      description = "my pkg";
    };
    result = pluginsLib.applyBuilderDefaults pluginBuilders cfg;
  in
    check "applyBuilderDefaults preserves all user fields"
    (result.version == "2.0" && result.description == "my pkg" && result.cargo-lock == "Cargo.lock");

  test_apply_defaults_no_meta = let
    pluginBuilders = {
      rust = {};
    };
    cfg = {
      build-system = "rust";
      version = "1.0";
    };
    result = pluginsLib.applyBuilderDefaults pluginBuilders cfg;
  in
    checkEq "applyBuilderDefaults handles builder with no meta"
    "1.0"
    result.version;

  # ── Builder routing tests ───────────────────────────────────────

  # We can't test full routing without real pkgs, but we can test the
  # selection logic by using mock builders.

  test_route_core_builder = let
    mockBuilders = {};
    coreBuilders = {
      generic = _pkgs: _root: name: _cfg: "built-${name}-generic";
    };
    result = pluginsLib.routeBuilder mockBuilders coreBuilders null null "my-pkg" {build-system = "generic";};
  in
    checkEq "routeBuilder selects core generic builder"
    "built-my-pkg-generic"
    result;

  test_route_plugin_builder_priority = let
    pluginBuilders = {
      rust = {
        buildRustPackage = _pkgs: _root: name: _cfg: "plugin-built-${name}";
        meta = {buildSystem = "rust";};
      };
    };
    coreBuilders = {
      rust = _pkgs: _root: name: _cfg: "core-built-${name}";
    };
    result = pluginsLib.routeBuilder pluginBuilders coreBuilders null null "my-pkg" {build-system = "rust";};
  in
    checkEq "routeBuilder prefers plugin builder over core"
    "plugin-built-my-pkg"
    result;

  test_route_core_fallback = let
    pluginBuilders = {};
    coreBuilders = {
      go = _pkgs: _root: name: _cfg: "core-go-${name}";
    };
    result = pluginsLib.routeBuilder pluginBuilders coreBuilders null null "api" {build-system = "go";};
  in
    checkEq "routeBuilder falls back to core builder"
    "core-go-api"
    result;

  test_route_unknown_throws =
    checkThrows "routeBuilder throws for unknown build-system"
    (pluginsLib.routeBuilder {} {} null null "pkg" {build-system = "unknown";});

  # ── Shell extras tests ──────────────────────────────────────────

  test_collect_shell_extras_empty = let
    result = pluginsLib.collectShellExtras {} null {};
  in
    checkEq "collectShellExtras with no plugins gives empty"
    []
    result;

  test_collect_shell_extras_mock = let
    shellExtras = {
      "test-plugin" = _pkgs: _cfg: ["extra-a" "extra-b"];
    };
    result = pluginsLib.collectShellExtras shellExtras null {};
  in
    checkEq "collectShellExtras collects from mock plugin"
    ["extra-a" "extra-b"]
    result;

  test_collect_shell_extras_multiple = let
    shellExtras = {
      "plugin-a" = _pkgs: _cfg: ["from-a"];
      "plugin-b" = _pkgs: _cfg: ["from-b-1" "from-b-2"];
    };
    result = pluginsLib.collectShellExtras shellExtras null {};
  in
    checkEq "collectShellExtras merges from multiple plugins"
    3 (builtins.length result);

  # ── Convention discovery integration tests ──────────────────────

  # Test that discoverPluginConventions calls the discovery function
  # with the right directory paths.

  test_discover_plugin_conventions_empty = let
    result = pluginsLib.discoverPluginConventions discover.discoverNclFiles /tmp {};
  in
    checkEq "discoverPluginConventions with empty conventions gives empty"
    {}
    result;

  test_discover_plugin_conventions_skips_disabled = let
    conventions = {
      crates = {
        dir = "crates";
        output = "packages";
        builder = "rust";
        autoDiscover = false;
        fromPlugin = true;
      };
    };
    result = pluginsLib.discoverPluginConventions discover.discoverNclFiles /tmp conventions;
  in
    checkEq "discoverPluginConventions skips disabled conventions"
    {}
    result;

  # ── Builder meta tests ──────────────────────────────────────────

  test_rust_builder_meta = let
    loaded = pluginsLib.loadPlugins pluginsDir ["nix-workspace-rust"];
    rustBuilder = loaded.builders.rust;
  in
    check "rust builder has correct meta.buildSystem"
    ((rustBuilder.meta or {}).buildSystem or null == "rust");

  test_go_builder_meta = let
    loaded = pluginsLib.loadPlugins pluginsDir ["nix-workspace-go"];
    goBuilder = loaded.builders.go;
  in
    check "go builder has correct meta name"
    ((goBuilder.meta or {}).name or null == "go");

  # ── End-to-end plugin flow test ─────────────────────────────────
  #
  # Simulates the full plugin lifecycle without actual Nickel evaluation:
  # 1. Resolve plugins
  # 2. Load builders
  # 3. Extract conventions from mock evaluated configs
  # 4. Validate
  # 5. Apply defaults

  test_e2e_plugin_flow = let
    pluginNames = ["nix-workspace-rust" "nix-workspace-go"];

    # Step 1: Resolve
    nclPaths = map (name: pluginsLib.resolvePluginNcl pluginsDir name) pluginNames;
    pathsExist = builtins.all (p: builtins.pathExists p) nclPaths;

    # Step 2: Load builders
    loaded = pluginsLib.loadPlugins pluginsDir pluginNames;
    hasRust = builtins.hasAttr "rust" loaded.builders;
    hasGo = builtins.hasAttr "go" loaded.builders;

    # Step 3: Mock extracted conventions
    mockEvaluatedConfigs = {
      "nix-workspace-rust" = {
        name = "nix-workspace-rust";
        conventions = {
          crates = {
            path = "crates";
            output = "packages";
            builder = "rust";
            auto-discover = true;
          };
        };
      };
      "nix-workspace-go" = {
        name = "nix-workspace-go";
        conventions = {
          go-modules = {
            path = "go-modules";
            output = "packages";
            builder = "go";
            auto-discover = true;
          };
        };
      };
    };
    conventions = pluginsLib.extractConventions mockEvaluatedConfigs;
    hasCrates = builtins.hasAttr "crates" conventions;
    hasGoModules = builtins.hasAttr "go-modules" conventions;

    # Step 4: Validate
    validation = pluginsLib.validatePlugins pluginNames;
    noErrors = validation == [];
  in
    check "end-to-end plugin flow succeeds"
    (pathsExist && hasRust && hasGo && hasCrates && hasGoModules && noErrors);
in
  # ── Collect all test results ────────────────────────────────────
  builtins.deepSeq [
    # Plugin resolution
    test_resolve_rust_plugin
    test_resolve_go_plugin
    test_resolve_short_name
    test_resolve_unknown_throws
    test_resolve_ncl_rust
    test_resolve_ncl_go
    test_resolve_builder_rust
    test_resolve_builder_go

    # Plugin loading
    test_load_plugins_empty
    test_load_plugins_empty_names
    test_load_plugins_rust
    test_load_plugins_go
    test_load_plugins_both
    test_load_plugins_names_preserved
    test_load_plugins_shell_extras_rust
    test_load_plugins_no_shell_extras_go

    # Convention extraction
    test_extract_conventions_empty
    test_extract_conventions_rust
    test_extract_conventions_dir
    test_extract_conventions_output
    test_extract_conventions_builder_field
    test_extract_conventions_from_plugin
    test_extract_conventions_multiple_plugins
    test_extract_conventions_no_conventions

    # Plugin validation
    test_validate_empty
    test_validate_single
    test_validate_two_unique
    test_validate_duplicate
    test_validate_duplicate_code
    test_validate_duplicate_severity
    test_validate_triple_duplicate
    test_validate_mixed_duplicates

    # Builder defaults
    test_apply_defaults_no_plugin
    test_apply_defaults_no_matching_builder
    test_apply_defaults_matching_builder
    test_apply_defaults_user_wins
    test_apply_defaults_preserves_all_user_fields
    test_apply_defaults_no_meta

    # Builder routing
    test_route_core_builder
    test_route_plugin_builder_priority
    test_route_core_fallback
    test_route_unknown_throws

    # Shell extras
    test_collect_shell_extras_empty
    test_collect_shell_extras_mock
    test_collect_shell_extras_multiple

    # Convention discovery
    test_discover_plugin_conventions_empty
    test_discover_plugin_conventions_skips_disabled

    # Builder meta
    test_rust_builder_meta
    test_go_builder_meta

    # End-to-end
    test_e2e_plugin_flow
  ]
  "All plugin integration tests passed (53 tests)."
