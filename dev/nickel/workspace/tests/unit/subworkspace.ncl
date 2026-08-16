# Unit tests for subworkspace-related contracts
#
# Validates that the WorkspaceConfig contract correctly handles the
# `dependencies` field for cross-subworkspace references, and that
# the DependencyRef contract (which is just Name) validates correctly.
#
# Run with: nickel eval tests/unit/subworkspace.ncl
let { WorkspaceConfig, DependencyRef, .. } = import "../../contracts/workspace.ncl" in
let { Name, .. } = import "../../contracts/common.ncl" in

# ── Helpers ───────────────────────────────────────────────────────
let passes = fun contract value =>
  let wrapper =
    std.contract.custom (fun label v =>
      std.contract.check contract label v
      |> match {
        'Ok _ => 'Ok true,
        'Error _ => 'Ok false,
      }
    )
  in
  (value | wrapper)
in

let fails = fun contract value =>
  !(passes contract value)
in

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

# ── DependencyRef ─────────────────────────────────────────────────
# DependencyRef is an alias for Name, so it must follow the same rules:
#   - Must start with a letter or underscore
#   - Only alphanumeric, hyphens, and underscores allowed
#   - Must not be empty
let dependency_ref_tests = {
  valid_simple = passes DependencyRef "mojo-zed",
  valid_underscore = passes DependencyRef "_internal",
  valid_hyphenated = passes DependencyRef "my-lib-core",
  valid_mixed = passes DependencyRef "Lib_A-2",
  valid_single_letter = passes DependencyRef "a",

  rejects_empty = fails DependencyRef "",
  rejects_starts_number = fails DependencyRef "1bad",
  rejects_starts_hyphen = fails DependencyRef "-bad",
  rejects_spaces = fails DependencyRef "has space",
  rejects_dots = fails DependencyRef "has.dot",
  rejects_slash = fails DependencyRef "has/slash",
  rejects_number = fails DependencyRef 42,
  rejects_bool = fails DependencyRef true,
  rejects_null = fails DependencyRef null,
  rejects_at_sign = fails DependencyRef "@scope/pkg",
}
in

# ── WorkspaceConfig with dependencies ─────────────────────────────
let test_workspace_no_dependencies =
  let config | WorkspaceConfig = {
    name = "root-workspace",
  }
  in
  [
    assert_ok "workspace without dependencies has empty default" (config.dependencies == {}),
    assert_export "workspace without dependencies exports" config,
  ]
in

let test_workspace_empty_dependencies =
  let config | WorkspaceConfig = {
    name = "root-workspace",
    dependencies = {},
  }
  in
  [
    assert_ok "workspace with empty dependencies" (config.dependencies == {}),
    assert_export "workspace with empty dependencies exports" config,
  ]
in

let test_workspace_single_dependency =
  let config | WorkspaceConfig = {
    name = "my-subworkspace",
    dependencies = {
      wasm = "mojo-wasm",
    },
  }
  in
  [
    assert_ok "workspace with single dependency has alias" (std.record.has_field "wasm" config.dependencies),
    assert_ok "workspace single dependency target" (config.dependencies.wasm == "mojo-wasm"),
    assert_export "workspace with single dependency exports" config,
  ]
in

let test_workspace_multiple_dependencies =
  let config | WorkspaceConfig = {
    name = "app-frontend",
    dependencies = {
      core = "shared-core",
      utils = "common-utils",
      api = "backend-api",
    },
  }
  in
  [
    assert_ok "workspace multiple deps has core" (std.record.has_field "core" config.dependencies),
    assert_ok "workspace multiple deps has utils" (std.record.has_field "utils" config.dependencies),
    assert_ok "workspace multiple deps has api" (std.record.has_field "api" config.dependencies),
    assert_ok "workspace multiple deps core value" (config.dependencies.core == "shared-core"),
    assert_ok "workspace multiple deps utils value" (config.dependencies.utils == "common-utils"),
    assert_ok "workspace multiple deps api value" (config.dependencies.api == "backend-api"),
    assert_export "workspace with multiple dependencies exports" config,
  ]
in

# ── Subworkspace-style configs (full workspace with typical subworkspace fields) ──
let test_subworkspace_with_packages_and_deps =
  let config | WorkspaceConfig = {
    name = "mojo-zed",
    description = "Mojo language support for Zed editor",
    systems = ["x86_64-linux", "aarch64-linux"],
    packages = {
      default = {
        description = "Mojo Zed extension",
        build-system = "generic",
      },
      lsp = {
        description = "Mojo LSP server",
        build-system = "rust",
      },
    },
    dependencies = {
      wasm = "mojo-wasm",
    },
  }
  in
  [
    assert_ok "subworkspace name" (config.name == "mojo-zed"),
    assert_ok "subworkspace has default package" (std.record.has_field "default" config.packages),
    assert_ok "subworkspace has lsp package" (std.record.has_field "lsp" config.packages),
    assert_ok "subworkspace has wasm dependency" (config.dependencies.wasm == "mojo-wasm"),
    assert_export "subworkspace with packages and deps exports" config,
  ]
in

let test_subworkspace_with_modules_and_deps =
  let config | WorkspaceConfig = {
    name = "infra",
    modules = {
      networking = {
        description = "Shared networking module",
      },
    },
    dependencies = {
      secrets = "vault-secrets",
    },
  }
  in
  [
    assert_ok "subworkspace with modules has networking" (std.record.has_field "networking" config.modules),
    assert_ok "subworkspace with modules has secrets dep" (config.dependencies.secrets == "vault-secrets"),
    assert_export "subworkspace with modules and deps exports" config,
  ]
in

let test_subworkspace_with_machines =
  let config | WorkspaceConfig = {
    name = "servers",
    machines = {
      web-server = {
        system = "x86_64-linux",
        state-version = "25.05",
        modules = ["nginx"],
      },
    },
    dependencies = {
      certs = "tls-certs",
      monitoring = "prometheus-stack",
    },
  }
  in
  [
    assert_ok "subworkspace with machines" (std.record.has_field "web-server" config.machines),
    assert_ok "subworkspace machine system" (config.machines.web-server.system == "x86_64-linux"),
    assert_ok "subworkspace has certs dep" (config.dependencies.certs == "tls-certs"),
    assert_ok "subworkspace has monitoring dep" (config.dependencies.monitoring == "prometheus-stack"),
    assert_export "subworkspace with machines exports" config,
  ]
in

# ── Dependency names with various valid formats ───────────────────
let test_dependency_name_formats =
  let config | WorkspaceConfig = {
    name = "test-workspace",
    dependencies = {
      simple = "lib",
      hyphenated = "my-lib",
      underscored = "my_lib",
      mixed = "My-Lib_v2",
      single = "a",
      leading_underscore = "_private",
    },
  }
  in
  [
    assert_ok "dep name simple" (config.dependencies.simple == "lib"),
    assert_ok "dep name hyphenated" (config.dependencies.hyphenated == "my-lib"),
    assert_ok "dep name underscored" (config.dependencies.underscored == "my_lib"),
    assert_ok "dep name mixed" (config.dependencies.mixed == "My-Lib_v2"),
    assert_ok "dep name single" (config.dependencies.single == "a"),
    assert_ok "dep name leading underscore" (config.dependencies.leading_underscore == "_private"),
    assert_export "dependency name formats export" config,
  ]
in

# ── Full round-trip: root workspace with subworkspace-like structure ──
let test_monorepo_root =
  let config | WorkspaceConfig = {
    name = "my-monorepo",
    description = "A monorepo with multiple subworkspaces",
    systems = ["x86_64-linux", "aarch64-linux"],
    nixpkgs = {
      allow-unfree = false,
    },
    packages = {
      shared-lib = {
        description = "Shared library used by all subworkspaces",
        build-system = "generic",
      },
    },
    shells = {
      default = {
        packages = ["git", "nickel"],
      },
    },
  }
  in
  [
    assert_ok "monorepo root name" (config.name == "my-monorepo"),
    assert_ok "monorepo root has shared-lib" (std.record.has_field "shared-lib" config.packages),
    assert_ok "monorepo root has default shell" (std.record.has_field "default" config.shells),
    assert_ok "monorepo root no dependencies" (config.dependencies == {}),
    assert_export "monorepo root exports" config,
  ]
in

let test_monorepo_subworkspace =
  let config | WorkspaceConfig = {
    name = "backend",
    description = "Backend service",
    systems = ["x86_64-linux"],
    packages = {
      default = {
        description = "Backend service binary",
        build-system = "go",
      },
      migrate = {
        description = "Database migration tool",
        build-system = "go",
      },
    },
    shells = {
      default = {
        packages = ["go", "gopls"],
      },
    },
    dependencies = {
      proto = "protobuf-schemas",
      shared = "shared-lib",
    },
  }
  in
  [
    assert_ok "monorepo sub name" (config.name == "backend"),
    assert_ok "monorepo sub has default pkg" (std.record.has_field "default" config.packages),
    assert_ok "monorepo sub has migrate pkg" (std.record.has_field "migrate" config.packages),
    assert_ok "monorepo sub default shell" (std.record.has_field "default" config.shells),
    assert_ok "monorepo sub has proto dep" (config.dependencies.proto == "protobuf-schemas"),
    assert_ok "monorepo sub has shared dep" (config.dependencies.shared == "shared-lib"),
    assert_export "monorepo subworkspace exports" config,
  ]
in

# ── Collect all results ──────────────────────────────────────────
let all_validator_tests =
  std.record.to_array dependency_ref_tests
in

let validator_failures =
  all_validator_tests
  |> std.array.filter (fun entry => entry.value == false)
  |> std.array.map (fun entry => entry.field)
in

let all_assert_results =
  test_workspace_no_dependencies
  @ test_workspace_empty_dependencies
  @ test_workspace_single_dependency
  @ test_workspace_multiple_dependencies
  @ test_subworkspace_with_packages_and_deps
  @ test_subworkspace_with_modules_and_deps
  @ test_subworkspace_with_machines
  @ test_dependency_name_formats
  @ test_monorepo_root
  @ test_monorepo_subworkspace
in

let total_count =
  std.array.length all_validator_tests
  + std.array.length all_assert_results
in

if std.array.length validator_failures > 0 then
  std.fail_with "FAILED validator tests: %{std.string.join ", " validator_failures}"
else
  let _ = all_assert_results |> std.array.map (fun r => std.trace r null) in
  "All %{std.to_string total_count} subworkspace contract tests passed."
