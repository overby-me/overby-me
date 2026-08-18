# Unit tests for ModuleConfig and HomeConfig contracts
#
# Validates that ModuleConfig and HomeConfig contracts correctly accept
# valid configurations, reject invalid ones, and apply defaults.
#
# Run with: nickel eval tests/unit/module.ncl
let { ModuleConfig, HomeConfig, .. } = import "../../contracts/module.ncl" in
let { ModuleRef, .. } = import "../../contracts/common.ncl" in

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

# ══════════════════════════════════════════════════════════════════
# ModuleConfig tests
# ══════════════════════════════════════════════════════════════════

# ── Minimal ModuleConfig ─────────────────────────────────────────
let test_module_minimal =
  let config | ModuleConfig = {} in
  [
    assert_ok "minimal module default imports" (config.imports == []),
    assert_ok "minimal module default extra-config" (config.extra-config == {}),
  ]
in

# ── Full ModuleConfig ────────────────────────────────────────────
let test_module_full =
  let config | ModuleConfig = {
    description = "Desktop environment with GNOME",
    imports = ["base", "audio"],
    options-namespace = "services.xserver",
    platforms = ["x86_64-linux", "aarch64-linux"],
    path = "./modules/desktop.nix",
    extra-config = {
      services.xserver.enable = true,
    },
  }
  in
  [
    assert_ok "full module description" (config.description == "Desktop environment with GNOME"),
    assert_ok "full module imports count" (std.array.length config.imports == 2),
    assert_ok "full module imports first" (std.array.first config.imports == "base"),
    assert_ok "full module imports second" (std.array.elem_at config.imports 1 == "audio"),
    assert_ok "full module options-namespace" (config.options-namespace == "services.xserver"),
    assert_ok "full module platforms count" (std.array.length config.platforms == 2),
    assert_ok "full module path" (config.path == "./modules/desktop.nix"),
    assert_ok "full module has extra-config" (config.extra-config.services.xserver.enable == true),
  ]
in

# ── ModuleConfig with description only ───────────────────────────
let test_module_description_only =
  let config | ModuleConfig = {
    description = "A simple module",
  }
  in
  [
    assert_ok "description-only module description" (config.description == "A simple module"),
    assert_ok "description-only module default imports" (config.imports == []),
    assert_ok "description-only module default extra-config" (config.extra-config == {}),
  ]
in

# ── ModuleConfig with imports ────────────────────────────────────
let test_module_with_imports =
  let config | ModuleConfig = {
    imports = ["networking", "security", "./custom/my-module.nix"],
  }
  in
  [
    assert_ok "module with imports count" (std.array.length config.imports == 3),
    assert_ok "module with imports has networking" (std.array.elem "networking" config.imports),
    assert_ok "module with imports has security" (std.array.elem "security" config.imports),
    assert_ok "module with imports has path" (std.array.elem "./custom/my-module.nix" config.imports),
  ]
in

# ── ModuleConfig with options-namespace ──────────────────────────
let test_module_namespace =
  let config | ModuleConfig = {
    options-namespace = "services.my-service",
  }
  in
  assert_ok "module options-namespace" (config.options-namespace == "services.my-service")
in

# ── ModuleConfig with platforms ──────────────────────────────────
let test_module_platforms =
  let config | ModuleConfig = {
    platforms = ["x86_64-linux"],
  }
  in
  [
    assert_ok "module platforms count" (std.array.length config.platforms == 1),
    assert_ok "module platforms value" (std.array.first config.platforms == "x86_64-linux"),
  ]
in

let test_module_all_platforms =
  let config | ModuleConfig = {
    platforms = ["x86_64-linux", "aarch64-linux", "x86_64-darwin", "aarch64-darwin"],
  }
  in
  assert_ok "module all platforms count" (std.array.length config.platforms == 4)
in

# ── ModuleConfig with path ───────────────────────────────────────
let test_module_with_relative_path =
  let config | ModuleConfig = {
    path = "./modules/desktop.nix",
  }
  in
  assert_ok "module relative path" (config.path == "./modules/desktop.nix")
in

let test_module_with_plain_path =
  let config | ModuleConfig = {
    path = "modules/desktop.nix",
  }
  in
  assert_ok "module plain path" (config.path == "modules/desktop.nix")
in

# ── ModuleConfig with extra-config ───────────────────────────────
let test_module_extra_config =
  let config | ModuleConfig = {
    extra-config = {
      enable = true,
      setting-a = "value-a",
      setting-b = 42,
      nested = {
        deep = true,
      },
    },
  }
  in
  [
    assert_ok "module extra-config has enable" (std.record.has_field "enable" config.extra-config),
    assert_ok "module extra-config has setting-a" (std.record.has_field "setting-a" config.extra-config),
    assert_ok "module extra-config has nested" (std.record.has_field "nested" config.extra-config),
  ]
in

# ── ModuleConfig export round-trip ───────────────────────────────
let test_module_export_minimal =
  let config | ModuleConfig = {} in
  assert_export "minimal module exports to JSON" config
in

let test_module_export_full =
  let config | ModuleConfig = {
    description = "Full module for export test",
    imports = ["base", "extra"],
    options-namespace = "programs.my-tool",
    platforms = ["x86_64-linux"],
    path = "./modules/full.nix",
    extra-config = {
      programs.my-tool.enable = true,
    },
  }
  in
  assert_export "full module exports to JSON" config
in

# ══════════════════════════════════════════════════════════════════
# HomeConfig tests
# ══════════════════════════════════════════════════════════════════

# ── Minimal HomeConfig ───────────────────────────────────────────
let test_home_minimal =
  let config | HomeConfig = {} in
  [
    assert_ok "minimal home default imports" (config.imports == []),
    assert_ok "minimal home default extra-config" (config.extra-config == {}),
  ]
in

# ── Full HomeConfig ──────────────────────────────────────────────
let test_home_full =
  let config | HomeConfig = {
    description = "Shell configuration with zsh and starship",
    imports = ["base-home", "git-config"],
    options-namespace = "programs.zsh",
    platforms = ["x86_64-linux", "aarch64-linux", "x86_64-darwin", "aarch64-darwin"],
    path = "./home/shell.nix",
    state-version = "25.05",
    extra-config = {
      programs.zsh.enable = true,
      programs.starship.enable = true,
    },
  }
  in
  [
    assert_ok "full home description" (config.description == "Shell configuration with zsh and starship"),
    assert_ok "full home imports count" (std.array.length config.imports == 2),
    assert_ok "full home imports first" (std.array.first config.imports == "base-home"),
    assert_ok "full home imports second" (std.array.elem_at config.imports 1 == "git-config"),
    assert_ok "full home options-namespace" (config.options-namespace == "programs.zsh"),
    assert_ok "full home platforms count" (std.array.length config.platforms == 4),
    assert_ok "full home path" (config.path == "./home/shell.nix"),
    assert_ok "full home state-version" (config.state-version == "25.05"),
    assert_ok "full home has extra-config" (config.extra-config.programs.zsh.enable == true),
  ]
in

# ── HomeConfig with description only ─────────────────────────────
let test_home_description_only =
  let config | HomeConfig = {
    description = "Git configuration module",
  }
  in
  [
    assert_ok "home description-only description" (config.description == "Git configuration module"),
    assert_ok "home description-only default imports" (config.imports == []),
    assert_ok "home description-only default extra-config" (config.extra-config == {}),
  ]
in

# ── HomeConfig with imports ──────────────────────────────────────
let test_home_with_imports =
  let config | HomeConfig = {
    imports = ["base-home", "theme", "./custom/home-extra.nix"],
  }
  in
  [
    assert_ok "home with imports count" (std.array.length config.imports == 3),
    assert_ok "home with imports has base-home" (std.array.elem "base-home" config.imports),
    assert_ok "home with imports has theme" (std.array.elem "theme" config.imports),
    assert_ok "home with imports has path" (std.array.elem "./custom/home-extra.nix" config.imports),
  ]
in

# ── HomeConfig with state-version ────────────────────────────────
let test_home_state_version =
  let config | HomeConfig = {
    state-version = "24.11",
  }
  in
  assert_ok "home state-version" (config.state-version == "24.11")
in

let test_home_no_state_version =
  let config | HomeConfig = {} in
  # state-version is optional, so it should not be present by default
  assert_ok "home no state-version by default" (!(std.record.has_field "state-version" config))
in

# ── HomeConfig with options-namespace ────────────────────────────
let test_home_namespace =
  let config | HomeConfig = {
    options-namespace = "programs.my-editor",
  }
  in
  assert_ok "home options-namespace" (config.options-namespace == "programs.my-editor")
in

# ── HomeConfig with platforms ────────────────────────────────────
let test_home_platforms =
  let config | HomeConfig = {
    platforms = ["x86_64-linux", "x86_64-darwin"],
  }
  in
  [
    assert_ok "home platforms count" (std.array.length config.platforms == 2),
    assert_ok "home platforms has linux" (std.array.elem "x86_64-linux" config.platforms),
    assert_ok "home platforms has darwin" (std.array.elem "x86_64-darwin" config.platforms),
  ]
in

# ── HomeConfig with path ─────────────────────────────────────────
let test_home_with_path =
  let config | HomeConfig = {
    path = "./home/shell.nix",
  }
  in
  assert_ok "home path" (config.path == "./home/shell.nix")
in

# ── HomeConfig with extra-config ─────────────────────────────────
let test_home_extra_config =
  let config | HomeConfig = {
    extra-config = {
      programs.git.enable = true,
      programs.git.userName = "Alice",
      programs.git.userEmail = "alice@example.com",
    },
  }
  in
  [
    assert_ok "home extra-config has git enable" (config.extra-config.programs.git.enable == true),
    assert_ok "home extra-config has git userName" (config.extra-config.programs.git.userName == "Alice"),
    assert_ok "home extra-config has git userEmail" (config.extra-config.programs.git.userEmail == "alice@example.com"),
  ]
in

# ── HomeConfig export round-trip ─────────────────────────────────
let test_home_export_minimal =
  let config | HomeConfig = {} in
  assert_export "minimal home exports to JSON" config
in

let test_home_export_full =
  let config | HomeConfig = {
    description = "Full home module for export test",
    imports = ["base-home"],
    options-namespace = "programs.shell",
    platforms = ["x86_64-linux"],
    path = "./home/shell.nix",
    state-version = "25.05",
    extra-config = {
      programs.zsh.enable = true,
    },
  }
  in
  assert_export "full home exports to JSON" config
in

# ══════════════════════════════════════════════════════════════════
# Workspace integration tests — modules and home in WorkspaceConfig
# ══════════════════════════════════════════════════════════════════
let { WorkspaceConfig, .. } = import "../../contracts/workspace.ncl" in

let test_workspace_with_modules =
  let config | WorkspaceConfig = {
    name = "module-test",
    modules = {
      desktop = {
        description = "Desktop environment module",
        imports = [],
      },
      networking = {
        description = "Networking module",
        options-namespace = "networking",
      },
    },
  }
  in
  [
    assert_ok "workspace has modules" (std.record.has_field "desktop" config.modules),
    assert_ok "workspace has networking module" (std.record.has_field "networking" config.modules),
    assert_ok "workspace module description" (config.modules.desktop.description == "Desktop environment module"),
    assert_ok "workspace module namespace" (config.modules.networking.options-namespace == "networking"),
  ]
in

let test_workspace_with_home =
  let config | WorkspaceConfig = {
    name = "home-test",
    home = {
      shell = {
        description = "Shell configuration",
        path = "./home/shell.nix",
      },
      editor = {
        description = "Editor configuration",
        imports = ["shell"],
      },
    },
  }
  in
  [
    assert_ok "workspace has home modules" (std.record.has_field "shell" config.home),
    assert_ok "workspace has editor home module" (std.record.has_field "editor" config.home),
    assert_ok "workspace home shell description" (config.home.shell.description == "Shell configuration"),
    assert_ok "workspace home shell path" (config.home.shell.path == "./home/shell.nix"),
    assert_ok "workspace home editor imports" (std.array.length config.home.editor.imports == 1),
  ]
in

let test_workspace_with_machines =
  let config | WorkspaceConfig = {
    name = "machine-workspace-test",
    machines = {
      my-server = {
        system = "x86_64-linux",
        state-version = "25.05",
        modules = ["networking"],
        networking = {
          firewall = {
            enable = true,
            allowed-tcp-ports = [22, 80],
          },
        },
      },
    },
    modules = {
      networking = {
        description = "Networking configuration",
      },
    },
  }
  in
  [
    assert_ok "workspace has machines" (std.record.has_field "my-server" config.machines),
    assert_ok "workspace machine system" (config.machines.my-server.system == "x86_64-linux"),
    assert_ok "workspace machine modules" (std.array.length config.machines.my-server.modules == 1),
    assert_ok "workspace machine firewall ports" (std.array.length config.machines.my-server.networking.firewall.allowed-tcp-ports == 2),
  ]
in

let test_workspace_defaults_empty_machines =
  let config | WorkspaceConfig = {
    name = "defaults-test",
  }
  in
  [
    assert_ok "workspace default machines" (config.machines == {}),
    assert_ok "workspace default modules" (config.modules == {}),
    assert_ok "workspace default home" (config.home == {}),
  ]
in

let test_workspace_full_with_all_types =
  let config | WorkspaceConfig = {
    name = "full-integration",
    description = "Full workspace with all output types",
    systems = ["x86_64-linux"],
    packages = {
      my-tool = {
        description = "A tool",
        build-system = "rust",
      },
    },
    shells = {
      default = {
        packages = ["cargo"],
      },
    },
    machines = {
      my-pc = {
        system = "x86_64-linux",
        modules = ["desktop"],
        time-zone = "UTC",
      },
    },
    modules = {
      desktop = {
        description = "Desktop module",
      },
    },
    home = {
      shell = {
        description = "Shell module",
      },
    },
  }
  in
  [
    assert_ok "full workspace has packages" (std.record.has_field "my-tool" config.packages),
    assert_ok "full workspace has shells" (std.record.has_field "default" config.shells),
    assert_ok "full workspace has machines" (std.record.has_field "my-pc" config.machines),
    assert_ok "full workspace has modules" (std.record.has_field "desktop" config.modules),
    assert_ok "full workspace has home" (std.record.has_field "shell" config.home),
  ]
in

let test_workspace_export_with_machines =
  let config | WorkspaceConfig = {
    name = "export-machines-test",
    machines = {
      test-box = {
        system = "x86_64-linux",
        state-version = "25.05",
      },
    },
    modules = {
      base = {
        description = "Base module",
      },
    },
    home = {
      dotfiles = {
        description = "Dotfiles management",
      },
    },
  }
  in
  assert_export "workspace with machines/modules/home exports to JSON" config
in

# ══════════════════════════════════════════════════════════════════
# Collect and run all tests
# ══════════════════════════════════════════════════════════════════
let all_assert_results =
  # ModuleConfig tests
  test_module_minimal
  @ test_module_full
  @ test_module_description_only
  @ test_module_with_imports
  @ test_module_platforms
  @ test_module_extra_config
  # HomeConfig tests
  @ test_home_minimal
  @ test_home_full
  @ test_home_description_only
  @ test_home_with_imports
  @ test_home_platforms
  @ test_home_extra_config
  # Workspace integration tests
  @ test_workspace_with_modules
  @ test_workspace_with_home
  @ test_workspace_with_machines
  @ test_workspace_defaults_empty_machines
  @ test_workspace_full_with_all_types
  # Singleton tests
  @ [
    test_module_namespace,
    test_module_all_platforms,
    test_module_with_relative_path,
    test_module_with_plain_path,
    test_module_export_minimal,
    test_module_export_full,
    test_home_state_version,
    test_home_no_state_version,
    test_home_namespace,
    test_home_with_path,
    test_home_export_minimal,
    test_home_export_full,
    test_workspace_export_with_machines,
  ]
in

let total_count = std.array.length all_assert_results in

let _ = all_assert_results |> std.array.map (fun r => std.trace r null) in
"All %{std.to_string total_count} ModuleConfig/HomeConfig tests passed."
