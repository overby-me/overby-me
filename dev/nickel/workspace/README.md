# nix-workspace

A Nickel-powered workspace manager for Nix flakes.

`nix-workspace` replaces [flakelight](https://github.com/accelbread/flakelight) and similar flake frameworks with a configuration layer built on [Nickel](https://nickel-lang.org/). It leverages Nickel's contract system and gradual typing to provide validated, well-documented workspace configuration with clear error messages — for both humans and AI agents.

## How it works

```text
┌─────────────────┐     Nickel      ┌──────────────┐      JSON       ┌─────────────┐
│  workspace.ncl  │ ──evaluate──▶   │  Validated    │ ──export──▶    │  Nix library │
│  (user config)  │                 │  config tree  │                │  (builders)  │
└─────────────────┘                 └──────────────┘                └──────┬──────┘
                                                                          │
                                                                          ▼
                                                                   ┌─────────────┐
                                                                   │ Flake       │
                                                                   │ outputs     │
                                                                   └─────────────┘
```

1. **Nickel layer** — Defines workspace structure. Contracts validate all configuration. Exports a JSON-serializable config tree.
2. **Nix layer** — A library consumes the evaluated config and produces standard flake outputs using nixpkgs builders.
3. **Flake shim** — A thin `flake.nix` that calls `nix-workspace` with the workspace root.

## Quick start

### 1. Create `flake.nix`

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-workspace.url = "git+https://tangled.org/@overby.me/overby.me?dir=nix-workspace";
  };

  outputs = inputs:
    inputs.nix-workspace ./. {
      inherit inputs;
    };
}
```

### 2. Create `workspace.ncl`

```nickel
{
  name = "my-project",
  description = "My Nix workspace",

  systems = ["x86_64-linux", "aarch64-linux"],

  nixpkgs = {
    allow-unfree = true,
  },

  packages = {
    my-tool = {
      src = "./src",
      build-system = "rust",
      description = "A CLI tool",
    },
  },

  shells = {
    default = {
      packages = ["cargo", "rustc", "rust-analyzer"],
    },
  },
}
```

### 3. Build and develop

```shell
nix build .#my-tool
nix develop
```

### Alternative: Use the CLI

```shell
# Install the CLI
nix profile install "git+https://tangled.org/@overby.me/overby.me?dir=nix-workspace"

# Initialize a new workspace
nix-workspace init my-project
cd my-project

# Validate configuration
nix-workspace check

# Show workspace structure
nix-workspace info

# Build a package
nix-workspace build my-tool

# Enter a dev shell
nix-workspace shell
```

## Directory conventions

Place `.ncl` files in convention directories and they are auto-discovered:

| Directory      | Flake output                          | Description                  |
|----------------|---------------------------------------|------------------------------|
| `packages/`    | `packages.<system>.<name>`            | Package definitions          |
| `shells/`      | `devShells.<system>.<name>`           | Development shells           |
| `machines/`    | `nixosConfigurations.<name>`          | NixOS machine configs        |
| `modules/`     | `nixosModules.<name>`                 | NixOS modules                |
| `home/`        | `homeModules.<name>`                  | Home-manager modules         |
| `overlays/`    | `overlays.<name>`                     | Nixpkgs overlays             |
| `lib/`         | `lib.<name>`                          | Library functions            |
| `templates/`   | `templates.<name>`                    | Flake templates              |
| `checks/`      | `checks.<system>.<name>`             | CI checks                    |

Convention directories are configurable:

```nickel
{
  conventions = {
    packages.path = "pkgs",           # Use pkgs/ instead of packages/
    overlays.auto-discover = false,   # Disable auto-discovery for overlays
  },
}
```

## CLI

The `nix-workspace` CLI provides a standalone interface for working with workspaces without writing Nix code directly.

### Commands

| Command | Description |
|---------|-------------|
| `nix-workspace init [path]` | Initialize a new workspace with scaffold files |
| `nix-workspace check` | Validate `workspace.ncl` against contracts |
| `nix-workspace info` | Show workspace structure and discovered outputs |
| `nix-workspace build [name]` | Build a package (delegates to `nix build`) |
| `nix-workspace shell [name]` | Enter a dev shell (delegates to `nix develop`) |

### Global flags

| Flag | Description |
|------|-------------|
| `--format human\|json` | Output format (default: `human`) |
| `--workspace-dir DIR` | Override workspace root directory |

### JSON diagnostic output

All commands support `--format json` for machine-parseable output, following the structured diagnostics schema from the spec:

```shell
nix-workspace check --format json
```

```json
{
  "diagnostics": [
    {
      "code": "NW001",
      "severity": "error",
      "file": "workspace.ncl",
      "line": 3,
      "message": "Expected System, got \"x86-linux\"",
      "hint": "Did you mean \"x86_64-linux\"?"
    }
  ]
}
```

### On-the-fly flake generation

If a workspace has `workspace.ncl` but no `flake.nix`, the CLI generates a temporary flake automatically when running `build` or `shell` commands:

```shell
# Works even without a flake.nix!
nix-workspace build hello
nix-workspace shell
```

The generated flake reference can be customized via the `NIX_WORKSPACE_FLAKE_REF` environment variable.

### Init options

```shell
# Initialize with specific systems and plugins
nix-workspace init my-project \
  --systems x86_64-linux,aarch64-linux,aarch64-darwin \
  --plugins nix-workspace-rust \
  --conventions packages,shells,modules

# Initialize without generating a flake.nix (standalone mode)
nix-workspace init my-project --no-flake
```

### Environment variables

| Variable | Description |
|----------|-------------|
| `NIX_WORKSPACE_CONTRACTS` | Path to the nix-workspace contracts directory |
| `NIX_WORKSPACE_PLUGINS` | Path to the nix-workspace plugins directory |
| `NIX_WORKSPACE_FLAKE_REF` | Flake reference for on-the-fly generation |

## System multiplexing

Declare systems once — they apply everywhere:

```nickel
{
  systems = ["x86_64-linux", "aarch64-linux"],

  packages = {
    my-tool = {
      build-system = "rust",
    },
    linux-only = {
      systems = ["x86_64-linux"],   # Override for this package only
      build-system = "rust",
    },
  },
}
```

You never write `packages.x86_64-linux.my-tool` — you write `packages.my-tool` and the system dimension is managed for you.

## NixOS machines

Define NixOS machine configurations directly in `workspace.ncl` or auto-discover them from the `machines/` directory:

```nickel
{
  name = "my-infra",

  machines = {
    workstation = {
      system = "x86_64-linux",
      state-version = "25.05",

      modules = ["desktop"],   # References modules/ directory

      host-name = "my-workstation",
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

      networking = {
        firewall = {
          enable = true,
          allowed-tcp-ports = [22, 80, 443],
        },
      },

      users = {
        alice = {
          extra-groups = ["wheel", "docker", "video"],
          home-manager = true,
          home-modules = ["shell", "editor"],
        },
      },
    },
  },
}
```

Each machine entry produces a `nixosConfigurations.<name>` flake output built with `nixpkgs.lib.nixosSystem`.

### Machine config fields

| Field           | Type                         | Default          | Description                                    |
|-----------------|------------------------------|------------------|------------------------------------------------|
| `system`        | `System`                     | _(required)_     | Target architecture                            |
| `state-version` | `StateVersion`               | `"25.05"`        | NixOS state version (`"YY.MM"`)                |
| `modules`       | `Array ModuleRef`            | `[]`             | NixOS modules to include                       |
| `host-name`     | `String`                     | machine name     | Hostname                                       |
| `special-args`  | `{ _ : Dyn }`               | `{}`             | Extra args passed to NixOS modules             |
| `users`         | `{ _ : UserConfig }`        | `{}`             | Per-user configurations                        |
| `boot-loader`   | `'systemd-boot \| 'grub \| 'none` | `'systemd-boot` | Boot loader to configure                |
| `file-systems`  | `{ _ : FileSystemConfig }`  | `{}`             | Mount points and devices                       |
| `networking`    | `NetworkingConfig`           | `{}`             | Networking and firewall settings               |
| `time-zone`     | `String`                     | _(optional)_     | Time zone (e.g. `"Europe/Copenhagen"`)         |
| `locale`        | `String`                     | _(optional)_     | Default locale (e.g. `"en_US.UTF-8"`)          |
| `extra-config`  | `{ _ : Dyn }`               | `{}`             | Escape hatch: raw NixOS config options         |

### Usage

```shell
# Build the system configuration
nix build .#nixosConfigurations.workstation.config.system.build.toplevel

# Switch to the new configuration
sudo nixos-rebuild switch --flake .#workstation
```

## NixOS modules

NixOS modules can be declared in `workspace.ncl` or auto-discovered from the `modules/` directory. Each module has two parts:

- **`modules/<name>.ncl`** — Nickel config (metadata, imports, validation)
- **`modules/<name>.nix`** — NixOS module implementation

```nickel
# modules/desktop.ncl
{
  description = "Desktop environment with GNOME",
  imports = [],
  options-namespace = "services.xserver",
}
```

```nix
# modules/desktop.nix
{ config, lib, pkgs, ... }: {
  services.xserver.enable = true;
  services.xserver.desktopManager.gnome.enable = true;
  # ...
}
```

Modules are referenced by name in machine configs (`modules = ["desktop"]`) and are exposed as `nixosModules.<name>` flake outputs.

### Module config fields

| Field               | Type              | Default      | Description                              |
|---------------------|-------------------|--------------|------------------------------------------|
| `description`       | `String`          | _(optional)_ | Human-readable description               |
| `imports`           | `Array ModuleRef` | `[]`         | Other modules this module depends on     |
| `options-namespace` | `String`          | _(optional)_ | NixOS option path (e.g. `"services.x"`)  |
| `platforms`         | `Array String`    | _(optional)_ | Compatible systems                       |
| `path`              | `String`          | _(optional)_ | Explicit path to the `.nix` module file  |
| `extra-config`      | `{ _ : Dyn }`    | `{}`         | Additional config merged into the module |

## Home-manager modules

Home-manager modules follow the same pattern as NixOS modules but live in the `home/` directory and are exposed as `homeModules.<name>` flake outputs.

```nickel
# In workspace.ncl or home/shell.ncl
{
  home = {
    shell = {
      description = "ZSH shell configuration",
      path = "./home/shell.nix",
    },
    editor = {
      description = "Neovim editor configuration",
      imports = ["shell"],
    },
  },
}
```

Home modules are referenced in machine user configs:

```nickel
{
  machines = {
    my-pc = {
      system = "x86_64-linux",
      users = {
        alice = {
          home-manager = true,
          home-modules = ["shell", "editor"],
        },
      },
    },
  },
}
```

## Contracts

`nix-workspace` ships Nickel contracts that validate your configuration and provide clear error messages:

```text
error: contract broken by the value of `system`
       invalid system "x86-linux"
   ┌─ contracts/machine.ncl:39:9
   │
39 │       | System
   │         ------ expected type
   │
   ┌─ machines/my-machine.ncl:3:13
   │
 3 │   system = "x86-linux",
   │            ^^^^^^^^^^^ applied to this expression
   │
   = Valid systems: x86_64-linux, aarch64-linux, x86_64-darwin, aarch64-darwin
```

```text
error: contract broken by the value of `state-version`
       invalid state version "unstable"
   │
   = State version must match the pattern "YY.MM" (e.g. "24.11", "25.05").
   = This corresponds to the NixOS release version.
```

```text
error: missing definition for `system`
   ┌─ contracts/machine.ncl:38:5
   │
38 │     system
   │     ^^^^^^ required here
```

### Contract hierarchy

```text
Workspace
├── WorkspaceConfig              # Top-level workspace.ncl structure
├── NixpkgsConfig                # nixpkgs settings
├── ConventionConfig             # Directory convention override
│
├── PackageConfig                # Package definition
├── ShellConfig                  # Development shell
│
├── MachineConfig                # NixOS machine configuration
│   ├── UserConfig               # Per-user settings (home-manager, groups)
│   ├── FileSystemConfig         # File system mount points
│   ├── NetworkingConfig         # Networking settings
│   │   ├── FirewallConfig       # Firewall rules
│   │   └── InterfaceConfig      # Network interface settings
│   └── StateVersion             # NixOS release version ("YY.MM")
│
├── ModuleConfig                 # NixOS module definition
├── HomeConfig                   # Home-manager module definition
│
├── OverlayConfig                # Nixpkgs overlay definition
├── CheckConfig                  # CI check definition
├── TemplateConfig               # Flake template definition
│
├── PluginConfig                 # Plugin definition contract
│   ├── PluginConvention         # Convention directory mapping
│   └── PluginBuilder            # Builder metadata + defaults
│
└── Common
    ├── System                   # Valid Nix system triple
    ├── Name                     # Valid derivation name
    ├── NonEmptyString           # Non-empty string
    ├── RelativePath             # Relative file path
    └── ModuleRef                # Module name or path reference
```

### Standalone validation

You can validate your workspace configuration without building anything:

```shell
nickel typecheck workspace.ncl
nickel export workspace.ncl    # Produces validated JSON
```

## Package build systems

The `build-system` field selects the Nix builder:

| Value       | Nix builder                              |
|-------------|------------------------------------------|
| `"generic"` | `stdenv.mkDerivation` (default)          |
| `"rust"`    | `rustPlatform.buildRustPackage`           |
| `"go"`      | `buildGoModule`                           |

### Package config fields

```nickel
{
  src = "./src",                           # Source directory (relative)
  build-system = "generic",                # Builder to use
  description = "My package",              # Human-readable description
  systems = ["x86_64-linux"],              # Override workspace systems
  build-inputs = ["openssl", "zlib"],      # Runtime dependencies
  native-build-inputs = ["pkg-config"],    # Build-time dependencies
  env = { MY_VAR = "value" },             # Build environment variables
  meta = {                                 # Package metadata
    homepage = "https://example.com",
    license = "MIT",
  },
  override = { },                          # Escape hatch: raw Nix attrs
}
```

## Shell config fields

```nickel
{
  packages = ["cargo", "rustc"],           # Packages in the shell
  env = { RUST_LOG = "debug" },            # Environment variables
  shell-hook = "echo hello",               # Script to run on entry
  tools = { rust-analyzer = "" },          # Tool specifications
  systems = ["x86_64-linux"],              # Override workspace systems
  inputs-from = ["my-tool"],               # Include build inputs from packages
}
```

## Diagnostic codes

Diagnostic codes are prefixed `NW` (nix-workspace) and grouped by category:

| Range | Category |
|-------|----------|
| `NW0xx` | Contract violations (type/value errors) |
| `NW1xx` | Discovery errors (missing files, bad directory structure) |
| `NW2xx` | Namespace conflicts (duplicate names, invalid derivation names) |
| `NW3xx` | Module errors (missing dependencies, circular imports) |
| `NW4xx` | System/plugin errors (unsupported system, missing input) |
| `NW5xx` | CLI errors (missing tool, tool failure) |

## Project structure

```text
nix-workspace/
├── SPEC.md                    # Full specification
├── README.md                  # This file
├── flake.nix                  # Project flake (exposes the library)
│
├── lib/                       # Nix library
│   ├── default.nix            # Main entry point (mkWorkspace)
│   ├── discover.nix           # Directory auto-discovery
│   ├── systems.nix            # System multiplexing
│   ├── eval-nickel.nix        # Nickel evaluation via IFD
│   └── builders/
│       ├── packages.nix       # Package builder
│       ├── shells.nix         # Shell builder
│       ├── machines.nix       # NixOS machine builder
│       └── modules.nix        # NixOS/home-manager module builder
│
├── contracts/                 # Nickel contracts
│   ├── workspace.ncl          # WorkspaceConfig contract
│   ├── package.ncl            # PackageConfig contract
│   ├── shell.ncl              # ShellConfig contract
│   ├── machine.ncl            # MachineConfig contract
│   ├── module.ncl             # ModuleConfig + HomeConfig contracts
│   └── common.ncl             # Shared types (System, Name, etc.)
│
├── examples/
│   ├── minimal/               # Minimal workspace example
│   │   ├── flake.nix
│   │   ├── workspace.ncl
│   │   └── packages/
│   │       └── hello.ncl
│   └── nixos/                 # NixOS machine configuration example
│       ├── flake.nix
│       ├── workspace.ncl
│       ├── machines/
│       │   └── my-machine.ncl
│       └── modules/
│           ├── desktop.ncl
│           └── desktop.nix
│
└── tests/
    ├── unit/                  # Nickel contract unit tests
    │   ├── common.ncl         # 44 tests — System, Name, etc.
    │   ├── package.ncl        # PackageConfig tests
    │   ├── machine.ncl        # 93 tests — MachineConfig, UserConfig, etc.
    │   ├── module.ncl         # 80 tests — ModuleConfig, HomeConfig
    │   └── workspace.ncl      # 82 tests — Full workspace validation
    └── errors/                # Error message snapshot tests
        ├── invalid-system.ncl
        ├── invalid-build-system.ncl
        ├── missing-field.ncl
        ├── invalid-machine-system.ncl
        ├── invalid-state-version.ncl
        └── missing-machine-system.ncl
```

## Development

```shell
# Enter the dev shell
nix develop

# Run all checks (contracts, integration tests, CLI tests)
nix flake check

# Validate contracts manually
nickel typecheck contracts/common.ncl
nickel typecheck contracts/package.ncl
nickel typecheck contracts/shell.ncl
nickel typecheck contracts/machine.ncl
nickel typecheck contracts/module.ncl
nickel typecheck contracts/workspace.ncl

# Run unit tests
nickel eval tests/unit/common.ncl      # 44 tests
nickel eval tests/unit/package.ncl     # PackageConfig tests
nickel eval tests/unit/machine.ncl     # 93 tests
nickel eval tests/unit/module.ncl      # 80 tests
nickel eval tests/unit/workspace.ncl   # 82 tests

# Run CLI tests
cd cli && cargo test                   # 77 tests

# Build the CLI
cd cli && cargo build
# or via Nix:
nix build
```

## Documentation

Detailed guides are available in the `docs/` directory:

| Document | Description |
|----------|-------------|
| [Error Catalog](docs/error-catalog.md) | Comprehensive reference for all `NWxxx` diagnostic codes |
| [Migration Guide](docs/migration-guide.md) | Migrating from flakelight or flake-parts |
| [Editor Integration](docs/editor-integration.md) | LSP setup for VS Code, Neovim, Helix, Zed, and Emacs |
| [CI Integration](docs/ci-integration.md) | GitHub Actions, GitLab CI, and generic CI setup |
| [Stability Guarantees](docs/stability.md) | Contract and API stability commitments from v1.0 |

## Roadmap

See [SPEC.md](./SPEC.md) for the full specification and milestone details.

- **v0.1 — Foundation** — Core contracts, package/shell discovery, system multiplexing ✅
- **v0.2 — NixOS integration** — Machine and module configs, home-manager support ✅
- **v0.3 — Subworkspaces** — Monorepo support with auto-namespacing ✅
- **v0.4 — Plugin system** — Extensible build systems and conventions ✅
- **v0.5 — Standalone CLI** — `nix-workspace init`, `check`, `build`, `shell` ✅
- **v1.0 — Production ready** — Full coverage, migration guides, editor integration ✅

## License

See [LICENSE](../LICENSE).