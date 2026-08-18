# Migration Guide

This guide helps you migrate existing Nix flake projects to nix-workspace from two popular frameworks: **flakelight** and **flake-parts**.

## Table of Contents

- [Why migrate?](#why-migrate)
- [Key differences](#key-differences)
- [From flakelight](#from-flakelight)
  - [Basic project](#flakelight-basic-project)
  - [Packages](#flakelight-packages)
  - [Dev shells](#flakelight-dev-shells)
  - [NixOS modules](#flakelight-nixos-modules)
  - [Overlays](#flakelight-overlays)
  - [Checks](#flakelight-checks)
- [From flake-parts](#from-flake-parts)
  - [Basic project](#flake-parts-basic-project)
  - [Packages](#flake-parts-packages)
  - [Dev shells](#flake-parts-dev-shells)
  - [NixOS configurations](#flake-parts-nixos-configurations)
  - [Overlays](#flake-parts-overlays)
  - [Checks](#flake-parts-checks)
- [Common patterns](#common-patterns)
  - [Multiple packages](#multiple-packages)
  - [System multiplexing](#system-multiplexing)
  - [Subworkspaces (monorepos)](#subworkspaces-monorepos)
  - [Plugins](#plugins)
- [Gradual migration](#gradual-migration)
- [Troubleshooting](#troubleshooting)

---

## Why migrate?

| Benefit | Details |
|---------|---------|
| **Validated configuration** | Nickel contracts catch misconfigurations before `nix build` runs — no more cryptic Nix stack traces for typos. |
| **Convention-over-configuration** | Drop a `.ncl` file into `packages/`, `shells/`, or `machines/` and it's automatically discovered. |
| **Structured diagnostics** | JSON error output (`--format json`) integrates with editors, CI, and AI agents. |
| **Subworkspace support** | Native monorepo support with automatic namespacing and dependency resolution. |
| **Simpler flake.nix** | Your `flake.nix` shrinks to ~10 lines regardless of project complexity. |

---

## Key differences

| Concept | flakelight | flake-parts | nix-workspace |
|---------|-----------|-------------|---------------|
| Config language | Nix | Nix | Nickel (`.ncl`) |
| Config file | `flake.nix` (inline) | `flake.nix` (inline) | `workspace.ncl` + convention dirs |
| Type checking | Nix module types | Nix module types | Nickel contracts |
| System handling | Automatic | `perSystem` module | Automatic (`systems` field) |
| Auto-discovery | `nixDir` + aliases | Manual | Convention directories |
| Error messages | Nix stack traces | Nix stack traces | Structured + readable |
| Plugin system | Flakelight modules | flake-parts modules | Nickel plugins |

---

## From flakelight

### Flakelight: Basic project

**Before (flakelight):**

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flakelight.url = "github:nix-community/flakelight";
  };

  outputs = { flakelight, ... }@inputs:
    flakelight ./. {
      inherit inputs;
      systems = [ "x86_64-linux" "aarch64-linux" ];
    };
}
```

**After (nix-workspace):**

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-workspace.url = "github:example/nix-workspace";  # replace with real URL
  };

  outputs = inputs:
    inputs.nix-workspace ./. {
      inherit inputs;
    };
}
```

```nickel
# workspace.ncl
{
  name = "my-project",
  systems = ["x86_64-linux", "aarch64-linux"],
}
```

### Flakelight: Packages

**Before:**

```nix
# flake.nix (flakelight)
flakelight ./. {
  package = { stdenv, lib, ... }:
    stdenv.mkDerivation {
      pname = "my-tool";
      version = "1.0.0";
      src = ./.;
      # ...
    };

  # Or multiple packages:
  packages = {
    foo = { stdenv, ... }: stdenv.mkDerivation { ... };
    bar = { stdenv, ... }: stdenv.mkDerivation { ... };
  };
}
```

**After:**

```nickel
# packages/my-tool.ncl
{
  description = "My tool",
  src = "./src",
  build-inputs = ["zlib"],
  native-build-inputs = ["cmake"],
}
```

For Rust packages, use the plugin:

```nickel
# workspace.ncl
{
  name = "my-project",
  plugins = ["nix-workspace-rust"],
}
```

```nickel
# packages/my-tool.ncl
{
  build-system = "rust",
  description = "A Rust CLI tool",
  src = ".",
}
```

### Flakelight: Dev shells

**Before:**

```nix
# flake.nix (flakelight)
flakelight ./. {
  devShell = {
    packages = pkgs: [ pkgs.cargo pkgs.rustc pkgs.rust-analyzer ];
    env.RUST_LOG = "debug";
    shellHook = ''
      echo "Welcome to my dev shell"
    '';
  };
}
```

**After:**

```nickel
# shells/default.ncl
{
  packages = ["cargo", "rustc", "rust-analyzer"],
  env = {
    RUST_LOG = "debug",
  },
  shell-hook = "echo 'Welcome to my dev shell'",
}
```

### Flakelight: NixOS modules

**Before:**

```nix
# flake.nix (flakelight)
flakelight ./. {
  nixosModules.my-service = { config, lib, pkgs, ... }: {
    options.services.my-service = {
      enable = lib.mkEnableOption "my-service";
    };
    config = lib.mkIf config.services.my-service.enable {
      # ...
    };
  };
}
```

**After:**

Create the module implementation as a `.nix` file (NixOS modules remain in Nix — only the metadata/discovery is in Nickel):

```nix
# modules/my-service.nix
{ config, lib, pkgs, ... }: {
  options.services.my-service = {
    enable = lib.mkEnableOption "my-service";
  };
  config = lib.mkIf config.services.my-service.enable {
    # ...
  };
}
```

Optionally add a Nickel config for metadata:

```nickel
# modules/my-service.ncl
{
  description = "My service module",
  options-namespace = "services.my-service",
}
```

The module is auto-discovered and becomes `nixosModules.my-service`.

### Flakelight: Overlays

**Before:**

```nix
# flake.nix (flakelight)
flakelight ./. {
  overlays.default = final: prev: {
    my-tool = final.callPackage ./pkgs/my-tool.nix {};
  };

  overlays.patched = final: prev: {
    openssl = prev.openssl.overrideAttrs (old: {
      # patches...
    });
  };
}
```

**After:**

```nix
# overlays/default.nix
final: prev: {
  my-tool = final.callPackage ./pkgs/my-tool.nix {};
}
```

```nix
# overlays/patched.nix
final: prev: {
  openssl = prev.openssl.overrideAttrs (old: {
    # patches...
  });
}
```

Optionally add Nickel metadata:

```nickel
# workspace.ncl
{
  name = "my-project",
  overlays = {
    default = {
      description = "Custom packages",
      path = "./overlays/default.nix",
      packages = ["my-tool"],
    },
    patched = {
      description = "Patched upstream packages",
      path = "./overlays/patched.nix",
      priority = 50,
      packages = ["openssl"],
    },
  },
}
```

Or let them be auto-discovered from the `overlays/` directory.

### Flakelight: Checks

**Before:**

```nix
# flake.nix (flakelight)
flakelight ./. {
  checks = {
    test = pkgs: pkgs.runCommand "test" {} ''
      ${pkgs.cargo}/bin/cargo test
      touch $out
    '';
    lint = pkgs: pkgs.runCommand "lint" {} ''
      ${pkgs.clippy}/bin/cargo clippy
      touch $out
    '';
  };
}
```

**After:**

```nickel
# workspace.ncl
{
  name = "my-project",
  checks = {
    test = {
      description = "Run unit tests",
      command = "cargo test --workspace",
      packages = ["cargo", "rustc"],
    },
    lint = {
      description = "Run clippy lints",
      command = "cargo clippy -- -D warnings",
      packages = ["cargo", "rustc", "clippy"],
    },
  },
}
```

Or use `.nix` files for complex checks:

```nix
# checks/test.nix
{ pkgs, workspaceRoot, lib }: pkgs.runCommand "test" {
  nativeBuildInputs = [ pkgs.cargo pkgs.rustc ];
  src = workspaceRoot;
} ''
  cp -r $src source && cd source
  cargo test --workspace
  touch $out
''
```

---

## From flake-parts

### Flake-parts: Basic project

**Before (flake-parts):**

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs = inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-linux" "aarch64-linux" ];

      perSystem = { pkgs, ... }: {
        # per-system outputs here
      };
    };
}
```

**After (nix-workspace):**

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-workspace.url = "github:example/nix-workspace";
  };

  outputs = inputs:
    inputs.nix-workspace ./. {
      inherit inputs;
    };
}
```

```nickel
# workspace.ncl
{
  name = "my-project",
  systems = ["x86_64-linux", "aarch64-linux"],
}
```

### Flake-parts: Packages

**Before:**

```nix
# flake.nix (flake-parts)
inputs.flake-parts.lib.mkFlake { inherit inputs; } {
  systems = [ "x86_64-linux" "aarch64-linux" ];

  perSystem = { pkgs, self', ... }: {
    packages = {
      my-tool = pkgs.stdenv.mkDerivation {
        pname = "my-tool";
        version = "1.0.0";
        src = ./.;
        buildInputs = [ pkgs.openssl ];
        nativeBuildInputs = [ pkgs.pkg-config ];
      };

      default = self'.packages.my-tool;
    };
  };
}
```

**After:**

```nickel
# packages/my-tool.ncl
{
  src = ".",
  description = "My tool",
  build-inputs = ["openssl"],
  native-build-inputs = ["pkg-config"],
}
```

> **Note:** When there's exactly one package and no explicit default shell, nix-workspace automatically creates a default dev shell with that package's build inputs. If you name a file `packages/default.ncl`, it becomes the `default` package output.

### Flake-parts: Dev shells

**Before:**

```nix
# flake.nix (flake-parts)
perSystem = { pkgs, ... }: {
  devShells.default = pkgs.mkShell {
    packages = with pkgs; [
      cargo rustc rust-analyzer clippy rustfmt
    ];

    RUST_LOG = "debug";

    shellHook = ''
      echo "Entering dev environment"
    '';
  };
};
```

**After:**

```nickel
# shells/default.ncl
{
  packages = ["cargo", "rustc", "rust-analyzer", "clippy", "rustfmt"],
  env = {
    RUST_LOG = "debug",
  },
  shell-hook = m%"
    echo "Entering dev environment"
  "%,
}
```

### Flake-parts: NixOS configurations

**Before:**

```nix
# flake.nix (flake-parts)
inputs.flake-parts.lib.mkFlake { inherit inputs; } {
  flake = {
    nixosConfigurations.my-machine = inputs.nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./hardware-configuration.nix
        ({ pkgs, ... }: {
          networking.hostName = "my-machine";
          system.stateVersion = "25.05";
          boot.loader.systemd-boot.enable = true;
          # ...
        })
      ];
    };
  };
};
```

**After:**

```nickel
# machines/my-machine.ncl
{
  system = "x86_64-linux",
  state-version = "25.05",
  host-name = "my-machine",
  boot-loader = 'systemd-boot,
  modules = ["./hardware-configuration.nix"],
  networking = {
    firewall = {
      enable = true,
    },
  },
}
```

> **Note:** Hardware-specific configuration (like `hardware-configuration.nix`) remains as plain Nix files. nix-workspace manages the high-level machine definition in Nickel but imports Nix modules directly.

### Flake-parts: Overlays

**Before:**

```nix
# flake.nix (flake-parts)
inputs.flake-parts.lib.mkFlake { inherit inputs; } {
  flake = {
    overlays.default = final: prev: {
      my-tool = final.callPackage ./platform/nix/config/my-tool.nix {};
    };
  };
};
```

**After:**

```nix
# overlays/default.nix
final: prev: {
  my-tool = final.callPackage ./platform/nix/config/my-tool.nix {};
}
```

The overlay is auto-discovered from the `overlays/` directory. No additional configuration needed.

### Flake-parts: Checks

**Before:**

```nix
# flake.nix (flake-parts)
perSystem = { pkgs, self', ... }: {
  checks = {
    test = pkgs.runCommand "test" {
      nativeBuildInputs = [ pkgs.cargo pkgs.rustc ];
      src = ./.;
    } ''
      cp -r $src source && cd source
      cargo test
      touch $out
    '';
  };
};
```

**After:**

```nickel
# workspace.ncl (or checks/test.ncl via auto-discovery)
{
  name = "my-project",
  checks = {
    test = {
      description = "Run unit tests",
      command = "cargo test",
      packages = ["cargo", "rustc"],
    },
  },
}
```

---

## Common patterns

### Multiple packages

With both flakelight and flake-parts, multiple packages are defined inline in `flake.nix`. With nix-workspace, each package gets its own file:

```text
packages/
├── cli.ncl
├── lib.ncl
└── server.ncl
```

Each file contains only that package's configuration — no boilerplate.

### System multiplexing

All three frameworks handle system multiplexing automatically. In nix-workspace, you declare target systems once in `workspace.ncl`:

```nickel
{
  name = "my-project",
  systems = ["x86_64-linux", "aarch64-linux", "x86_64-darwin", "aarch64-darwin"],
}
```

Individual packages or shells can override the systems list:

```nickel
# packages/linux-tool.ncl
{
  description = "Linux-only tool",
  systems = ["x86_64-linux", "aarch64-linux"],
}
```

### Subworkspaces (monorepos)

Neither flakelight nor flake-parts have native subworkspace support. With nix-workspace, any subdirectory containing a `workspace.ncl` is automatically discovered as a subworkspace:

```text
my-monorepo/
├── workspace.ncl          # root workspace
├── lib-core/
│   ├── workspace.ncl      # subworkspace
│   └── packages/
│       └── default.ncl    # → packages.lib-core
├── app-web/
│   ├── workspace.ncl      # subworkspace
│   └── packages/
│       └── default.ncl    # → packages.app-web
└── app-cli/
    ├── workspace.ncl      # subworkspace
    └── packages/
        └── default.ncl    # → packages.app-cli
```

Outputs are automatically namespaced with the subworkspace directory name as a prefix.

### Plugins

nix-workspace plugins replace language-specific flakelight modules and flake-parts modules:

```nickel
# workspace.ncl
{
  name = "my-project",
  plugins = ["nix-workspace-rust", "nix-workspace-go"],
}
```

Plugins can add:

- New contracts (e.g., `RustPackage` with Rust-specific fields)
- Convention directories (e.g., `crates/` for Rust)
- Builders (e.g., `buildRustPackage` wrapper)
- Extensions to existing contracts (e.g., adding `edition` to `PackageConfig`)

---

## Gradual migration

You don't have to migrate everything at once. Here's a recommended approach:

### Step 1: Create the skeleton

```bash
# In your existing project
nix-workspace init .
```

This creates `workspace.ncl` alongside your existing `flake.nix`.

### Step 2: Migrate packages first

Move package definitions from `flake.nix` to `packages/*.ncl` files. Validate with:

```bash
nix-workspace check
```

### Step 3: Switch flake.nix

Replace your flakelight/flake-parts `flake.nix` with the nix-workspace version:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nix-workspace.url = "github:example/nix-workspace";
  };

  outputs = inputs:
    inputs.nix-workspace ./. {
      inherit inputs;
    };
}
```

### Step 4: Migrate remaining outputs

Move dev shells, NixOS configs, modules, overlays, checks, and templates to their respective convention directories.

### Step 5: Verify

```bash
nix flake check
nix flake show
```

---

## Troubleshooting

### "Contract violation" errors

nix-workspace validates your configuration much earlier than flakelight or flake-parts. If you see contract violations:

1. Run `nix-workspace check --format json` for structured error output.
2. Check the field path in the error — it tells you exactly which config field is wrong.
3. Look up the error code in the [Error Catalog](./error-catalog.md) for resolution steps.

### Missing outputs

If `nix flake show` doesn't show expected outputs:

1. Check that `.ncl` files are in the correct convention directory (`packages/`, `shells/`, etc.).
2. Verify that `workspace.ncl` has the correct `systems` list.
3. Run `nix-workspace info` to see what was discovered.

### NixOS module migration

NixOS modules remain as `.nix` files — only the metadata and discovery layer uses Nickel. If a module doesn't appear in `nixosModules`:

1. Check that the `.nix` file is in the `modules/` directory.
2. Ensure the file is a valid NixOS module (a function taking `{ config, lib, pkgs, ... }`).
3. If the module has a companion `.ncl` file, check that the `path` field points to the correct `.nix` file.

### Overlays not applying

Overlays in nix-workspace are flake outputs (`overlays.<name>`), just like in flakelight and flake-parts. They are not automatically applied to your workspace's nixpkgs. To apply overlays to your workspace's nixpkgs:

```nickel
# workspace.ncl
{
  name = "my-project",
  nixpkgs = {
    overlays = ["self.overlays.default"],
  },
}
```

### Escape hatches

If nix-workspace's contracts are too restrictive for a specific use case, every config type has an `extra-config` field that passes data through verbatim:

```nickel
# packages/special.ncl
{
  description = "Special package",
  override = {
    # Arbitrary Nix attributes, passed through to the builder
    dontFixup = "true",
    postInstall = "echo done",
  },
}
```

For NixOS machines, use `extra-config`:

```nickel
# machines/special.ncl
{
  system = "x86_64-linux",
  extra-config = {
    # Arbitrary NixOS config options
    services.openssh.enable = true,
  },
}
```
