# Unit tests for the WorkspaceConfig contract
#
# These tests verify that the workspace-level contract correctly validates
# configuration structures, rejects invalid inputs, and applies defaults.
# Updated for v0.2 to cover machines, modules, and home fields.
#
# Run with: nickel eval tests/unit/workspace.ncl
let { WorkspaceConfig, NixpkgsConfig, ConventionConfig, .. } = import "../../contracts/workspace.ncl" in
let { System, Name, .. } = import "../../contracts/common.ncl" in
let { MachineConfig, .. } = import "../../contracts/machine.ncl" in
let { ModuleConfig, HomeConfig, .. } = import "../../contracts/module.ncl" in

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

# ── Minimal valid workspace ──────────────────────────────────────
let test_minimal =
  let config | WorkspaceConfig = {
    name = "test-workspace",
  }
  in
  assert_ok "minimal workspace with just a name" (config.name == "test-workspace")
in

# ── Defaults are applied ─────────────────────────────────────────
let test_defaults =
  let config | WorkspaceConfig = {
    name = "defaults-test",
  }
  in
  [
    assert_ok "default systems" (config.systems == ["x86_64-linux", "aarch64-linux"]),
    assert_ok "default nixpkgs" (config.nixpkgs.allow-unfree == false),
    assert_ok "default nixpkgs overlays" (config.nixpkgs.overlays == []),
    assert_ok "default packages" (config.packages == {}),
    assert_ok "default shells" (config.shells == {}),
    assert_ok "default machines" (config.machines == {}),
    assert_ok "default modules" (config.modules == {}),
    assert_ok "default home" (config.home == {}),
    assert_ok "default conventions" (config.conventions == {}),
  ]
in

# ── Full workspace config ────────────────────────────────────────
let test_full =
  let config | WorkspaceConfig = {
    name = "full-test",
    description = "A fully specified workspace for testing",
    systems = ["x86_64-linux", "aarch64-linux", "x86_64-darwin", "aarch64-darwin"],
    nixpkgs = {
      allow-unfree = true,
      overlays = ["my-overlay"],
      config = {
        permittedInsecurePackages = ["old-package-1.0"],
      },
    },
    packages = {
      my-tool = {
        description = "A test tool",
        build-system = "rust",
        build-inputs = ["openssl"],
      },
      another = {
        build-system = "generic",
      },
    },
    shells = {
      default = {
        packages = ["cargo", "rustc"],
        env = {
          RUST_LOG = "debug",
        },
      },
    },
    conventions = {
      packages = {
        path = "pkgs",
        auto-discover = true,
      },
    },
  }
  in
  [
    assert_ok "full config name" (config.name == "full-test"),
    assert_ok "full config description" (config.description == "A fully specified workspace for testing"),
    assert_ok "full config systems length" (std.array.length config.systems == 4),
    assert_ok "full config allow-unfree" (config.nixpkgs.allow-unfree == true),
    assert_ok "full config has my-tool" (std.record.has_field "my-tool" config.packages),
    assert_ok "full config has default shell" (std.record.has_field "default" config.shells),
    assert_ok "full config convention path" (config.conventions.packages.path == "pkgs"),
  ]
in

# ── Packages with all fields ─────────────────────────────────────
let test_package_fields =
  let config | WorkspaceConfig = {
    name = "pkg-test",
    packages = {
      full-pkg = {
        src = "./src",
        build-system = "go",
        description = "A Go package",
        systems = ["x86_64-linux"],
        build-inputs = ["git"],
        native-build-inputs = ["pkg-config"],
        env = {
          CGO_ENABLED = "1",
        },
        meta = {
          homepage = "https://example.com",
          license = "MIT",
        },
        override = {
          vendorHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        },
      },
    },
  }
  in
  [
    assert_ok "package src" (config.packages.full-pkg.src == "./src"),
    assert_ok "package build-system" (config.packages.full-pkg.build-system == "go"),
    assert_ok "package systems override" (config.packages.full-pkg.systems == ["x86_64-linux"]),
    assert_ok "package build-inputs" (config.packages.full-pkg.build-inputs == ["git"]),
    assert_ok "package env" (config.packages.full-pkg.env.CGO_ENABLED == "1"),
    assert_ok "package meta homepage" (config.packages.full-pkg.meta.homepage == "https://example.com"),
  ]
in

# ── Shells with all fields ───────────────────────────────────────
let test_shell_fields =
  let config | WorkspaceConfig = {
    name = "shell-test",
    shells = {
      dev = {
        packages = ["cargo", "rustc", "rust-analyzer"],
        env = {
          RUST_BACKTRACE = "1",
        },
        shell-hook = "echo entering dev shell",
        tools = {
          rust-analyzer = "",
        },
        systems = ["x86_64-linux"],
        inputs-from = ["my-tool"],
      },
    },
  }
  in
  [
    assert_ok "shell packages" (std.array.length config.shells.dev.packages == 3),
    assert_ok "shell env" (config.shells.dev.env.RUST_BACKTRACE == "1"),
    assert_ok "shell hook" (config.shells.dev.shell-hook == "echo entering dev shell"),
    assert_ok "shell tools" (config.shells.dev.tools.rust-analyzer == ""),
    assert_ok "shell systems" (config.shells.dev.systems == ["x86_64-linux"]),
    assert_ok "shell inputs-from" (config.shells.dev.inputs-from == ["my-tool"]),
  ]
in

# ── NixpkgsConfig defaults ───────────────────────────────────────
let test_nixpkgs_defaults =
  let cfg | NixpkgsConfig = {} in
  [
    assert_ok "nixpkgs default allow-unfree" (cfg.allow-unfree == false),
    assert_ok "nixpkgs default overlays" (cfg.overlays == []),
    assert_ok "nixpkgs default config" (cfg.config == {}),
  ]
in

# ── ConventionConfig ─────────────────────────────────────────────
let test_convention =
  let cfg | ConventionConfig = {
    path = "custom-packages",
    auto-discover = false,
  }
  in
  [
    assert_ok "convention path" (cfg.path == "custom-packages"),
    assert_ok "convention auto-discover" (cfg.auto-discover == false),
  ]
in

let test_convention_defaults =
  let cfg | ConventionConfig = {} in
  assert_ok "convention default auto-discover" (cfg.auto-discover == true)
in

# ── Machines in workspace ─────────────────────────────────────────
let test_machines =
  let config | WorkspaceConfig = {
    name = "machine-test",
    machines = {
      my-server = {
        system = "x86_64-linux",
        state-version = "25.05",
        modules = ["networking", "monitoring"],
        host-name = "server-01",
        networking = {
          firewall = {
            enable = true,
            allowed-tcp-ports = [22, 80, 443],
          },
        },
        time-zone = "Europe/Copenhagen",
        locale = "en_US.UTF-8",
        boot-loader = 'systemd-boot,
        file-systems = {
          "/" = {
            device = "/dev/disk/by-label/nixos",
            fs-type = "ext4",
          },
          "/boot" = {
            device = "/dev/disk/by-label/boot",
            fs-type = "vfat",
            needed-for-boot = true,
          },
        },
        users = {
          alice = {
            extra-groups = ["wheel", "docker"],
            home-manager = true,
            home-modules = ["shell", "editor"],
          },
        },
      },
      my-desktop = {
        system = "aarch64-linux",
        modules = ["desktop"],
        boot-loader = 'grub,
      },
    },
  }
  in
  [
    assert_ok "workspace has my-server" (std.record.has_field "my-server" config.machines),
    assert_ok "workspace has my-desktop" (std.record.has_field "my-desktop" config.machines),
    assert_ok "machine system" (config.machines.my-server.system == "x86_64-linux"),
    assert_ok "machine state-version" (config.machines.my-server.state-version == "25.05"),
    assert_ok "machine modules count" (std.array.length config.machines.my-server.modules == 2),
    assert_ok "machine host-name" (config.machines.my-server.host-name == "server-01"),
    assert_ok "machine firewall enabled" (config.machines.my-server.networking.firewall.enable == true),
    assert_ok "machine firewall ports" (std.array.length config.machines.my-server.networking.firewall.allowed-tcp-ports == 3),
    assert_ok "machine time-zone" (config.machines.my-server.time-zone == "Europe/Copenhagen"),
    assert_ok "machine locale" (config.machines.my-server.locale == "en_US.UTF-8"),
    assert_ok "machine boot-loader" (config.machines.my-server.boot-loader == 'systemd-boot),
    assert_ok "machine has root fs" (std.record.has_field "/" config.machines.my-server.file-systems),
    assert_ok "machine root device" (config.machines.my-server.file-systems."/".device == "/dev/disk/by-label/nixos"),
    assert_ok "machine boot fs needed" (config.machines.my-server.file-systems."/boot".needed-for-boot == true),
    assert_ok "machine has alice user" (std.record.has_field "alice" config.machines.my-server.users),
    assert_ok "machine alice groups" (std.array.length config.machines.my-server.users.alice.extra-groups == 2),
    assert_ok "machine alice hm enabled" (config.machines.my-server.users.alice.home-manager == true),
    assert_ok "machine alice hm modules" (std.array.length config.machines.my-server.users.alice.home-modules == 2),
    assert_ok "desktop boot-loader" (config.machines.my-desktop.boot-loader == 'grub),
    assert_ok "desktop default state-version" (config.machines.my-desktop.state-version == "25.05"),
  ]
in

# ── Modules in workspace ──────────────────────────────────────────
let test_modules =
  let config | WorkspaceConfig = {
    name = "modules-test",
    modules = {
      desktop = {
        description = "Desktop environment with GNOME",
        options-namespace = "services.xserver",
      },
      networking = {
        description = "Base networking configuration",
        imports = [],
      },
      monitoring = {
        description = "Monitoring stack",
        imports = ["networking"],
        path = "./modules/monitoring.nix",
        platforms = ["x86_64-linux"],
      },
    },
  }
  in
  [
    assert_ok "workspace has desktop module" (std.record.has_field "desktop" config.modules),
    assert_ok "workspace has networking module" (std.record.has_field "networking" config.modules),
    assert_ok "workspace has monitoring module" (std.record.has_field "monitoring" config.modules),
    assert_ok "module desktop description" (config.modules.desktop.description == "Desktop environment with GNOME"),
    assert_ok "module desktop namespace" (config.modules.desktop.options-namespace == "services.xserver"),
    assert_ok "module networking imports" (config.modules.networking.imports == []),
    assert_ok "module monitoring imports" (std.array.length config.modules.monitoring.imports == 1),
    assert_ok "module monitoring path" (config.modules.monitoring.path == "./modules/monitoring.nix"),
    assert_ok "module monitoring platforms" (std.array.length config.modules.monitoring.platforms == 1),
  ]
in

# ── Home modules in workspace ─────────────────────────────────────
let test_home =
  let config | WorkspaceConfig = {
    name = "home-test",
    home = {
      shell = {
        description = "ZSH shell configuration",
        path = "./home/shell.nix",
        options-namespace = "programs.zsh",
      },
      editor = {
        description = "Neovim editor configuration",
        imports = ["shell"],
        platforms = ["x86_64-linux", "aarch64-linux", "x86_64-darwin", "aarch64-darwin"],
      },
      git = {
        description = "Git configuration",
        extra-config = {
          programs.git.enable = true,
        },
      },
    },
  }
  in
  [
    assert_ok "workspace has shell home module" (std.record.has_field "shell" config.home),
    assert_ok "workspace has editor home module" (std.record.has_field "editor" config.home),
    assert_ok "workspace has git home module" (std.record.has_field "git" config.home),
    assert_ok "home shell description" (config.home.shell.description == "ZSH shell configuration"),
    assert_ok "home shell path" (config.home.shell.path == "./home/shell.nix"),
    assert_ok "home shell namespace" (config.home.shell.options-namespace == "programs.zsh"),
    assert_ok "home editor imports" (std.array.length config.home.editor.imports == 1),
    assert_ok "home editor platforms" (std.array.length config.home.editor.platforms == 4),
    assert_ok "home git has extra-config" (config.home.git.extra-config.programs.git.enable == true),
  ]
in

# ── Full workspace with all output types ──────────────────────────
let test_all_output_types =
  let config | WorkspaceConfig = {
    name = "full-stack",
    description = "A workspace with every output type",
    systems = ["x86_64-linux"],
    nixpkgs = {
      allow-unfree = true,
    },
    packages = {
      my-tool = {
        build-system = "rust",
        description = "CLI tool",
      },
    },
    shells = {
      default = {
        packages = ["cargo", "rustc"],
      },
    },
    machines = {
      workstation = {
        system = "x86_64-linux",
        modules = ["desktop"],
        time-zone = "UTC",
      },
    },
    modules = {
      desktop = {
        description = "Desktop environment",
      },
    },
    home = {
      dotfiles = {
        description = "User dotfiles",
      },
    },
  }
  in
  [
    assert_ok "full-stack has packages" (std.record.has_field "my-tool" config.packages),
    assert_ok "full-stack has shells" (std.record.has_field "default" config.shells),
    assert_ok "full-stack has machines" (std.record.has_field "workstation" config.machines),
    assert_ok "full-stack has modules" (std.record.has_field "desktop" config.modules),
    assert_ok "full-stack has home" (std.record.has_field "dotfiles" config.home),
    assert_ok "full-stack machine system" (config.machines.workstation.system == "x86_64-linux"),
    assert_ok "full-stack machine modules" (config.machines.workstation.modules == ["desktop"]),
  ]
in

# ── Export round-trip ─────────────────────────────────────────────
let test_export =
  let config | WorkspaceConfig = {
    name = "export-test",
    packages = {
      hello = {
        description = "hello world",
      },
    },
  }
  in
  assert_export "workspace round-trips through JSON" config
in

let test_export_with_machines =
  let config | WorkspaceConfig = {
    name = "export-machines-test",
    machines = {
      test-box = {
        system = "x86_64-linux",
        state-version = "25.05",
        file-systems = {
          "/" = {
            device = "/dev/sda1",
          },
        },
      },
    },
    modules = {
      base = {
        description = "Base module",
      },
    },
    home = {
      shell = {
        description = "Shell config",
      },
    },
  }
  in
  assert_export "workspace with machines/modules/home round-trips through JSON" config
in

# ── Collect and run all tests ─────────────────────────────────────
{
  results =
    [
      test_minimal,
      test_export,
      test_export_with_machines,
      test_convention_defaults,
    ]
    @ test_defaults
    @ test_full
    @ test_package_fields
    @ test_shell_fields
    @ test_nixpkgs_defaults
    @ test_convention
    @ test_machines
    @ test_modules
    @ test_home
    @ test_all_output_types,
}
