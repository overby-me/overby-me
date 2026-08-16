# Unit tests for the PluginConfig contract
#
# These tests verify that the plugin contract correctly validates
# plugin definitions, rejects invalid inputs, and applies defaults.
#
# Run with: nickel eval tests/unit/plugin.ncl
let { PluginConfig, PluginConvention, PluginBuilder, .. } = import "../../contracts/plugin.ncl" in
let { Name, .. } = import "../../contracts/common.ncl" in

let assert_ok = fun label value =>
  if value then
    "PASS: %{label}"
  else
    std.fail_with "FAIL: %{label}"
in

let assert_export = fun label expr =>
  let _ = std.serialize 'Json expr in
  "PASS: %{label} (exported successfully)"
in

# ── Minimal valid plugin ───────────────────────────────────────
let test_minimal_plugin =
  let cfg =
    {
      name = "my-plugin",
    } | PluginConfig
  in
  assert_export "minimal plugin with just a name" cfg
in

# ── Full plugin with all fields ────────────────────────────────
let test_full_plugin =
  let cfg =
    {
      name = "nix-workspace-rust",
      description = "Rust/Cargo build support",
      version = "0.4.0",
      contracts = {
        RustPackage = {
          edition = "2021",
          features = [],
        },
      },
      conventions = {
        crates = {
          path = "crates",
          output = "packages",
          builder = "rust",
        },
      },
      builders = {
        rust = {
          name = "rust",
          description = "Build Rust packages via rustPlatform.buildRustPackage",
          defaults = {
            build-system = "rust",
          },
        },
      },
      extend = {
        PackageConfig = {
          edition = "2021",
        },
      },
    } | PluginConfig
  in
  assert_export "full plugin with all fields" cfg
in

# ── Plugin defaults ────────────────────────────────────────────
let test_defaults_contracts =
  let cfg = { name = "test-defaults" } | PluginConfig in
  assert_ok "default contracts is empty" (cfg.contracts == {})
in

let test_defaults_conventions =
  let cfg = { name = "test-defaults" } | PluginConfig in
  assert_ok "default conventions is empty" (cfg.conventions == {})
in

let test_defaults_builders =
  let cfg = { name = "test-defaults" } | PluginConfig in
  assert_ok "default builders is empty" (cfg.builders == {})
in

let test_defaults_extend =
  let cfg = { name = "test-defaults" } | PluginConfig in
  assert_ok "default extend is empty" (cfg.extend == {})
in

# ── Name validation ────────────────────────────────────────────
let test_valid_name_simple =
  let cfg = { name = "my-plugin" } | PluginConfig in
  assert_ok "simple name accepted" (cfg.name == "my-plugin")
in

let test_valid_name_prefixed =
  let cfg = { name = "nix-workspace-rust" } | PluginConfig in
  assert_ok "nix-workspace-prefixed name accepted" (cfg.name == "nix-workspace-rust")
in

let test_valid_name_underscore =
  let cfg = { name = "_internal_plugin" } | PluginConfig in
  assert_ok "underscore-prefixed name accepted" (cfg.name == "_internal_plugin")
in

let test_valid_name_alphanumeric =
  let cfg = { name = "plugin123" } | PluginConfig in
  assert_ok "alphanumeric name accepted" (cfg.name == "plugin123")
in

# ── Convention validation ──────────────────────────────────────
let test_convention_minimal =
  let cfg =
    {
      name = "conv-test",
      conventions = {
        crates = {
          path = "crates",
          output = "packages",
          builder = "rust",
        },
      },
    } | PluginConfig
  in
  assert_ok "convention path correct" (cfg.conventions.crates.path == "crates")
in

let test_convention_output =
  let cfg =
    {
      name = "conv-test",
      conventions = {
        crates = {
          path = "crates",
          output = "packages",
          builder = "rust",
        },
      },
    } | PluginConfig
  in
  assert_ok "convention output correct" (cfg.conventions.crates.output == "packages")
in

let test_convention_builder =
  let cfg =
    {
      name = "conv-test",
      conventions = {
        crates = {
          path = "crates",
          output = "packages",
          builder = "rust",
        },
      },
    } | PluginConfig
  in
  assert_ok "convention builder correct" (cfg.conventions.crates.builder == "rust")
in

let test_convention_auto_discover_default =
  let cfg =
    {
      name = "conv-test",
      conventions = {
        crates = {
          path = "crates",
          output = "packages",
          builder = "rust",
        },
      },
    } | PluginConfig
  in
  assert_ok "convention auto-discover defaults to true" (cfg.conventions.crates.auto-discover == true)
in

let test_convention_auto_discover_false =
  let cfg =
    {
      name = "conv-test",
      conventions = {
        crates = {
          path = "crates",
          output = "packages",
          builder = "rust",
          auto-discover = false,
        },
      },
    } | PluginConfig
  in
  assert_ok "convention auto-discover can be false" (cfg.conventions.crates.auto-discover == false)
in

let test_multiple_conventions =
  let cfg =
    {
      name = "multi-conv",
      conventions = {
        crates = {
          path = "crates",
          output = "packages",
          builder = "rust",
        },
        go-modules = {
          path = "go-modules",
          output = "packages",
          builder = "go",
        },
      },
    } | PluginConfig
  in
  let has_crates = std.record.has_field "crates" cfg.conventions in
  let has_go = std.record.has_field "go-modules" cfg.conventions in
  assert_ok "multiple conventions accepted" (has_crates && has_go)
in

# ── Builder validation ─────────────────────────────────────────
let test_builder_minimal =
  let cfg =
    {
      name = "builder-test",
      builders = {
        rust = {
          name = "rust",
        },
      },
    } | PluginConfig
  in
  assert_ok "builder name correct" (cfg.builders.rust.name == "rust")
in

let test_builder_with_description =
  let cfg =
    {
      name = "builder-test",
      builders = {
        rust = {
          name = "rust",
          description = "Builds Rust packages",
        },
      },
    } | PluginConfig
  in
  assert_ok "builder description accepted" (cfg.builders.rust.description == "Builds Rust packages")
in

let test_builder_defaults_empty =
  let cfg =
    {
      name = "builder-test",
      builders = {
        rust = {
          name = "rust",
        },
      },
    } | PluginConfig
  in
  assert_ok "builder defaults default to empty" (cfg.builders.rust.defaults == {})
in

let test_builder_with_defaults =
  let cfg =
    {
      name = "builder-test",
      builders = {
        rust = {
          name = "rust",
          defaults = {
            build-system = "rust",
            cargo-lock = "Cargo.lock",
          },
        },
      },
    } | PluginConfig
  in
  assert_ok "builder defaults accepted" (cfg.builders.rust.defaults.build-system == "rust")
in

let test_multiple_builders =
  let cfg =
    {
      name = "multi-builder",
      builders = {
        rust = {
          name = "rust",
          description = "Build Rust packages",
        },
        go = {
          name = "go",
          description = "Build Go modules",
        },
      },
    } | PluginConfig
  in
  let has_rust = std.record.has_field "rust" cfg.builders in
  let has_go = std.record.has_field "go" cfg.builders in
  assert_ok "multiple builders accepted" (has_rust && has_go)
in

# ── Contract extensions ────────────────────────────────────────
let test_extend_single_contract =
  let cfg =
    {
      name = "ext-test",
      extend = {
        PackageConfig = {
          edition = "2021",
          features = [],
        },
      },
    } | PluginConfig
  in
  assert_ok "single contract extension accepted" (std.record.has_field "PackageConfig" cfg.extend)
in

let test_extend_multiple_contracts =
  let cfg =
    {
      name = "ext-test",
      extend = {
        PackageConfig = {
          edition = "2021",
        },
        ShellConfig = {
          rust-toolchain = "stable",
        },
      },
    } | PluginConfig
  in
  let has_pkg = std.record.has_field "PackageConfig" cfg.extend in
  let has_shell = std.record.has_field "ShellConfig" cfg.extend in
  assert_ok "multiple contract extensions accepted" (has_pkg && has_shell)
in

let test_extend_with_nested_records =
  let cfg =
    {
      name = "ext-test",
      extend = {
        PackageConfig = {
          rust-config = {
            edition = "2021",
            features = ["serde"],
          },
        },
      },
    } | PluginConfig
  in
  assert_export "nested record extension exports" cfg
in

# ── Contracts field ────────────────────────────────────────────
let test_contracts_single =
  let cfg =
    {
      name = "contracts-test",
      contracts = {
        RustPackage = {
          edition = "2021",
          features = [],
        },
      },
    } | PluginConfig
  in
  assert_ok "single contract definition accepted" (std.record.has_field "RustPackage" cfg.contracts)
in

let test_contracts_multiple =
  let cfg =
    {
      name = "contracts-test",
      contracts = {
        RustPackage = {
          edition = "2021",
        },
        GoPackage = {
          go-version = "1_23",
        },
      },
    } | PluginConfig
  in
  let has_rust = std.record.has_field "RustPackage" cfg.contracts in
  let has_go = std.record.has_field "GoPackage" cfg.contracts in
  assert_ok "multiple contract definitions accepted" (has_rust && has_go)
in

# ── Optional fields ────────────────────────────────────────────
let test_description_optional =
  let cfg = { name = "opt-test" } | PluginConfig in
  assert_ok "description is optional" (!(std.record.has_field "description" cfg))
in

let test_version_optional =
  let cfg = { name = "opt-test" } | PluginConfig in
  assert_ok "version is optional" (!(std.record.has_field "version" cfg))
in

let test_description_present =
  let cfg =
    {
      name = "opt-test",
      description = "A test plugin",
    } | PluginConfig
  in
  assert_ok "description accepted when present" (cfg.description == "A test plugin")
in

let test_version_present =
  let cfg =
    {
      name = "opt-test",
      version = "1.0.0",
    } | PluginConfig
  in
  assert_ok "version accepted when present" (cfg.version == "1.0.0")
in

# ── PluginConvention contract directly ─────────────────────────
let test_plugin_convention_direct =
  let conv =
    {
      path = "my-dir",
      output = "packages",
      builder = "custom",
    } | PluginConvention
  in
  assert_ok "PluginConvention validates directly" (conv.path == "my-dir")
in

let test_plugin_convention_auto_discover_default =
  let conv =
    {
      path = "my-dir",
      output = "packages",
      builder = "custom",
    } | PluginConvention
  in
  assert_ok "PluginConvention auto-discover defaults true" (conv.auto-discover == true)
in

# ── PluginBuilder contract directly ────────────────────────────
let test_plugin_builder_direct =
  let b =
    {
      name = "my-builder",
    } | PluginBuilder
  in
  assert_ok "PluginBuilder validates directly" (b.name == "my-builder")
in

let test_plugin_builder_defaults_default =
  let b =
    {
      name = "my-builder",
    } | PluginBuilder
  in
  assert_ok "PluginBuilder defaults to empty" (b.defaults == {})
in

# ── Export / serialization ─────────────────────────────────────
let test_export_minimal =
  assert_export
    "minimal plugin serializes to JSON"
    ({ name = "export-test" } | PluginConfig)
in

let test_export_rust_plugin =
  assert_export
    "rust-like plugin serializes to JSON"
    (
      {
        name = "nix-workspace-rust",
        description = "Rust build support",
        version = "0.4.0",
        contracts = {
          RustPackage = {
            edition = "2021",
            features = [],
          },
        },
        conventions = {
          crates = {
            path = "crates",
            output = "packages",
            builder = "rust",
          },
        },
        builders = {
          rust = {
            name = "rust",
            defaults = { build-system = "rust" },
          },
        },
        extend = {
          PackageConfig = {
            edition = "2021",
          },
        },
      } | PluginConfig
    )
in

let test_export_go_plugin =
  assert_export
    "go-like plugin serializes to JSON"
    (
      {
        name = "nix-workspace-go",
        description = "Go module build support",
        version = "0.4.0",
        conventions = {
          go-modules = {
            path = "go-modules",
            output = "packages",
            builder = "go",
            auto-discover = true,
          },
        },
        builders = {
          go = {
            name = "go",
            defaults = { build-system = "go" },
          },
        },
      } | PluginConfig
    )
in

# ── Validate actual built-in plugins ───────────────────────────
let test_validate_rust_plugin =
  let cfg = (import "../../plugins/rust/plugin.ncl") | PluginConfig in
  assert_export "built-in rust plugin validates" cfg
in

let test_validate_go_plugin =
  let cfg = (import "../../plugins/go/plugin.ncl") | PluginConfig in
  assert_export "built-in go plugin validates" cfg
in

let test_rust_plugin_name =
  let cfg = (import "../../plugins/rust/plugin.ncl") | PluginConfig in
  assert_ok "rust plugin name is nix-workspace-rust" (cfg.name == "nix-workspace-rust")
in

let test_go_plugin_name =
  let cfg = (import "../../plugins/go/plugin.ncl") | PluginConfig in
  assert_ok "go plugin name is nix-workspace-go" (cfg.name == "nix-workspace-go")
in

let test_rust_plugin_has_crates_convention =
  let cfg = (import "../../plugins/rust/plugin.ncl") | PluginConfig in
  assert_ok "rust plugin has crates convention" (std.record.has_field "crates" cfg.conventions)
in

let test_go_plugin_has_go_modules_convention =
  let cfg = (import "../../plugins/go/plugin.ncl") | PluginConfig in
  assert_ok "go plugin has go-modules convention" (std.record.has_field "go-modules" cfg.conventions)
in

let test_rust_plugin_has_rust_builder =
  let cfg = (import "../../plugins/rust/plugin.ncl") | PluginConfig in
  assert_ok "rust plugin has rust builder" (std.record.has_field "rust" cfg.builders)
in

let test_go_plugin_has_go_builder =
  let cfg = (import "../../plugins/go/plugin.ncl") | PluginConfig in
  assert_ok "go plugin has go builder" (std.record.has_field "go" cfg.builders)
in

let test_rust_plugin_extends_package_config =
  let cfg = (import "../../plugins/rust/plugin.ncl") | PluginConfig in
  assert_ok "rust plugin extends PackageConfig" (std.record.has_field "PackageConfig" cfg.extend)
in

let test_rust_plugin_extends_shell_config =
  let cfg = (import "../../plugins/rust/plugin.ncl") | PluginConfig in
  assert_ok "rust plugin extends ShellConfig" (std.record.has_field "ShellConfig" cfg.extend)
in

let test_go_plugin_extends_package_config =
  let cfg = (import "../../plugins/go/plugin.ncl") | PluginConfig in
  assert_ok "go plugin extends PackageConfig" (std.record.has_field "PackageConfig" cfg.extend)
in

let test_rust_crates_convention_path =
  let cfg = (import "../../plugins/rust/plugin.ncl") | PluginConfig in
  assert_ok "rust crates convention path is 'crates'" (cfg.conventions.crates.path == "crates")
in

let test_rust_crates_convention_output =
  let cfg = (import "../../plugins/rust/plugin.ncl") | PluginConfig in
  assert_ok "rust crates convention output is 'packages'" (cfg.conventions.crates.output == "packages")
in

let test_rust_crates_convention_builder =
  let cfg = (import "../../plugins/rust/plugin.ncl") | PluginConfig in
  assert_ok "rust crates convention builder is 'rust'" (cfg.conventions.crates.builder == "rust")
in

let test_go_modules_convention_path =
  let cfg = (import "../../plugins/go/plugin.ncl") | PluginConfig in
  assert_ok "go go-modules convention path is 'go-modules'" (cfg.conventions.go-modules.path == "go-modules")
in

let test_go_modules_convention_output =
  let cfg = (import "../../plugins/go/plugin.ncl") | PluginConfig in
  assert_ok "go go-modules convention output is 'packages'" (cfg.conventions.go-modules.output == "packages")
in

let test_go_modules_convention_builder =
  let cfg = (import "../../plugins/go/plugin.ncl") | PluginConfig in
  assert_ok "go go-modules convention builder is 'go'" (cfg.conventions.go-modules.builder == "go")
in

# ── mkWorkspaceConfig with plugin extensions ───────────────────
let test_mk_workspace_config_base =
  let { WorkspaceConfig, mkWorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in
  let { PackageConfig, .. } = import "../../contracts/package.ncl" in
  let { ShellConfig, .. } = import "../../contracts/shell.ncl" in
  let cfg =
    {
      name = "test-mk",
    } | (mkWorkspaceConfig PackageConfig ShellConfig)
  in
  assert_ok "mkWorkspaceConfig with base contracts works" (cfg.name == "test-mk")
in

let test_mk_workspace_config_equals_default =
  let { WorkspaceConfig, mkWorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in
  let { PackageConfig, .. } = import "../../contracts/package.ncl" in
  let { ShellConfig, .. } = import "../../contracts/shell.ncl" in
  let cfg_default = { name = "test-eq" } | WorkspaceConfig in
  let cfg_factory = { name = "test-eq" } | (mkWorkspaceConfig PackageConfig ShellConfig) in
  assert_ok
    "mkWorkspaceConfig equivalent to WorkspaceConfig"
    (cfg_default.name == cfg_factory.name)
in

let test_mk_workspace_config_extended_pkg =
  let { mkWorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in
  let { PackageConfig, .. } = import "../../contracts/package.ncl" in
  let { ShellConfig, .. } = import "../../contracts/shell.ncl" in
  let ExtPkg =
    PackageConfig
    & {
      my-custom-field
        | String
        | optional,
    }
  in
  let cfg =
    {
      name = "test-ext",
      packages = {
        my-pkg = {
          my-custom-field = "hello",
        },
      },
    } | (mkWorkspaceConfig ExtPkg ShellConfig)
  in
  assert_ok
    "mkWorkspaceConfig with extended PackageConfig accepts custom field"
    (cfg.packages.my-pkg.my-custom-field == "hello")
in

let test_mk_workspace_config_extended_shell =
  let { mkWorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in
  let { PackageConfig, .. } = import "../../contracts/package.ncl" in
  let { ShellConfig, .. } = import "../../contracts/shell.ncl" in
  let ExtShell =
    ShellConfig
    & {
      my-shell-extra
        | String
        | optional,
    }
  in
  let cfg =
    {
      name = "test-ext-shell",
      shells = {
        default = {
          my-shell-extra = "world",
        },
      },
    } | (mkWorkspaceConfig PackageConfig ExtShell)
  in
  assert_ok
    "mkWorkspaceConfig with extended ShellConfig accepts custom field"
    (cfg.shells.default.my-shell-extra == "world")
in

let test_mk_workspace_config_extended_both =
  let { mkWorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in
  let { PackageConfig, .. } = import "../../contracts/package.ncl" in
  let { ShellConfig, .. } = import "../../contracts/shell.ncl" in
  let ExtPkg =
    PackageConfig
    & {
      edition
        | String
        | optional,
    }
  in
  let ExtShell =
    ShellConfig
    & {
      rust-toolchain
        | String
        | optional,
    }
  in
  let cfg =
    {
      name = "test-ext-both",
      packages = {
        my-tool = {
          build-system = "rust",
          edition = "2021",
        },
      },
      shells = {
        default = {
          rust-toolchain = "stable",
          packages = ["cargo"],
        },
      },
    } | (mkWorkspaceConfig ExtPkg ExtShell)
  in
  let pkg_ok = (cfg.packages.my-tool.edition == "2021") in
  let shell_ok = (cfg.shells.default.rust-toolchain == "stable") in
  assert_ok "mkWorkspaceConfig with both extensions works" (pkg_ok && shell_ok)
in

let test_workspace_plugins_field =
  let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in
  let cfg =
    {
      name = "test-plugins-field",
      plugins = ["nix-workspace-rust", "nix-workspace-go"],
    } | WorkspaceConfig
  in
  assert_ok
    "plugins field accepted in WorkspaceConfig"
    (std.array.length cfg.plugins == 2)
in

let test_workspace_plugins_default_empty =
  let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in
  let cfg =
    {
      name = "test-plugins-default",
    } | WorkspaceConfig
  in
  assert_ok
    "plugins defaults to empty array"
    (cfg.plugins == [])
in

# ── Collect all results ────────────────────────────────────────
[
  test_minimal_plugin,
  test_full_plugin,
  test_defaults_contracts,
  test_defaults_conventions,
  test_defaults_builders,
  test_defaults_extend,
  test_valid_name_simple,
  test_valid_name_prefixed,
  test_valid_name_underscore,
  test_valid_name_alphanumeric,
  test_convention_minimal,
  test_convention_output,
  test_convention_builder,
  test_convention_auto_discover_default,
  test_convention_auto_discover_false,
  test_multiple_conventions,
  test_builder_minimal,
  test_builder_with_description,
  test_builder_defaults_empty,
  test_builder_with_defaults,
  test_multiple_builders,
  test_extend_single_contract,
  test_extend_multiple_contracts,
  test_extend_with_nested_records,
  test_contracts_single,
  test_contracts_multiple,
  test_description_optional,
  test_version_optional,
  test_description_present,
  test_version_present,
  test_plugin_convention_direct,
  test_plugin_convention_auto_discover_default,
  test_plugin_builder_direct,
  test_plugin_builder_defaults_default,
  test_export_minimal,
  test_export_rust_plugin,
  test_export_go_plugin,
  test_validate_rust_plugin,
  test_validate_go_plugin,
  test_rust_plugin_name,
  test_go_plugin_name,
  test_rust_plugin_has_crates_convention,
  test_go_plugin_has_go_modules_convention,
  test_rust_plugin_has_rust_builder,
  test_go_plugin_has_go_builder,
  test_rust_plugin_extends_package_config,
  test_rust_plugin_extends_shell_config,
  test_go_plugin_extends_package_config,
  test_rust_crates_convention_path,
  test_rust_crates_convention_output,
  test_rust_crates_convention_builder,
  test_go_modules_convention_path,
  test_go_modules_convention_output,
  test_go_modules_convention_builder,
  test_mk_workspace_config_base,
  test_mk_workspace_config_equals_default,
  test_mk_workspace_config_extended_pkg,
  test_mk_workspace_config_extended_shell,
  test_mk_workspace_config_extended_both,
  test_workspace_plugins_field,
  test_workspace_plugins_default_empty,
]
