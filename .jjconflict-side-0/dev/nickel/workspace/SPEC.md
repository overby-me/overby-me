# nix-workspace

A Nickel-powered workspace manager for Nix flakes.

## Overview

`nix-workspace` replaces [flakelight](https://github.com/accelbread/flakelight) and similar flake frameworks with a configuration layer built on [Nickel](https://nickel-lang.org/). It leverages Nickel's contract system and gradual typing to provide validated, well-documented workspace configuration with clear error messages — for both humans and AI agents.

The core idea: Nickel handles the *what* (declarative structure, validation, error reporting) and Nix handles the *how* (building derivations, evaluating modules). Users write `workspace.ncl` instead of complex Nix expressions, and `nix-workspace` produces standard flake outputs.

## Motivation

### Problems with Nix the language

- **No type system** — Configuration errors surface deep in evaluation as cryptic stack traces ("attribute 'x' missing at /nix/store/...-source/...nix:47:13") rather than at the point of misconfiguration.
- **Poor error messages** — Nix errors are positional, deeply nested, and lack context about *what the user was trying to do*. AI agents struggle to parse and act on them.
- **Boilerplate** — Every flake repeats the same `eachSystem`, `inputs.nixpkgs.legacyPackages`, and output-wiring patterns.
- **Footguns** — Namespace collisions, implicit behaviors, and the gap between "what you wrote" and "what Nix evaluated" cause subtle bugs.

### Why Nickel

- **Contracts** — Rich validation with custom error messages. A `NixosConfiguration` contract can tell you *exactly* which field is wrong and what it expected.
- **Gradual typing** — Start untyped, add contracts incrementally. No all-or-nothing commitment.
- **Merge semantics** — Nickel's record merging is well-defined and predictable, unlike Nix's `//` and `mkMerge` interactions.
- **Structured errors** — Nickel errors include the contract name, the expected type, and the offending value — machine-parseable and human-readable.

## Architecture

### Hybrid evaluation model

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
2. **Nix layer** — A library (`nix-workspace.lib`) consumes the evaluated config and produces standard flake outputs using nixpkgs builders, the NixOS module system, etc.
3. **Flake shim** — A thin `flake.nix` that calls `nix-workspace` with the workspace root, similar to how flakelight works today.

This hybrid avoids depending on the experimental `nickel-nix` bridge while getting Nickel's benefits where they matter most — the configuration surface.

### Evaluation phases

```text
Phase 1: Discovery
  Scan workspace root for workspace.ncl, subworkspaces, and convention directories.

Phase 2: Nickel evaluation
  Evaluate workspace.ncl with contracts applied.
  Merge subworkspace configs with automatic namespacing.
  Produce validated JSON config tree.

Phase 3: Nix construction
  Nix library reads JSON config.
  Builds flake outputs (packages, shells, NixOS configs, etc.).
  Applies system multiplexing.

Phase 4: Output
  Standard flake outputs, indistinguishable from a hand-written flake.
```

## User interface

### Flake integration (primary mode)

```nix
# flake.nix
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

### Workspace configuration

```nickel
# workspace.ncl
{
  name = "my-project",
  description = "My Nix workspace",

  systems = ["x86_64-linux", "aarch64-linux"],

  # Nixpkgs configuration
  nixpkgs = {
    allow-unfree = true,
  },

  # Auto-discovered from packages/ directory, or declared explicitly
  packages = {
    my-tool = {
      src = "./src",
      build-system = "rust",
      description = "A CLI tool",
    },
  },

  # Auto-discovered from machines/ directory
  machines = {
    gravitas = {
      system = "x86_64-linux",
      modules = ["desktop", "development"],
    },
  },

  # Auto-discovered from shells/ directory
  shells = {
    default = {
      packages = ["cargo", "rustc", "rust-analyzer"],
    },
  },
}
```

### Standalone mode (future)

```shell
# Initialize a new workspace
nix-workspace init

# Validate workspace configuration
nix-workspace check

# Show workspace structure
nix-workspace info

# Build a package
nix-workspace build my-tool

# Enter a dev shell
nix-workspace shell
```

The CLI tool would be written in Rust, using the Nickel library for evaluation. It can either delegate to `nix build` / `nix develop` under the hood or generate a `flake.nix` on the fly.

## Directory conventions

### Default directory mapping

| Directory      | Flake output                          | Description                          |
|----------------|---------------------------------------|--------------------------------------|
| `packages/`    | `packages.<system>.<name>`            | Package definitions                  |
| `shells/`      | `devShells.<system>.<name>`           | Development shells                   |
| `machines/`    | `nixosConfigurations.<name>`          | NixOS machine configurations         |
| `modules/`     | `nixosModules.<name>`                 | NixOS modules                        |
| `home/`        | `homeModules.<name>`                  | Home-manager modules                 |
| `overlays/`    | `overlays.<name>`                     | Nixpkgs overlays                     |
| `lib/`         | `lib.<name>`                          | Library functions                    |
| `templates/`   | `templates.<name>`                    | Flake templates                      |
| `checks/`      | `checks.<system>.<name>`             | CI checks                            |

These names are inspired by Cargo (`packages/`) and Deno (`modules/`) conventions rather than mapping 1:1 to Nix flake output attribute names. The mapping is configurable:

```nickel
# workspace.ncl
{
  conventions = {
    # Rename the packages directory
    packages.path = "pkgs",

    # Disable auto-discovery for overlays
    overlays.auto-discover = false,
  },
}
```

### Subworkspaces

Any directory containing a `workspace.ncl` is a subworkspace. The boundary is the `workspace.ncl` file itself — **not** the git repository structure. This means subworkspaces work equally well as:

- **Plain subdirectories** within a monorepo
- **Git submodules** pointing to external repositories
- **Checkouts from other VCS** (Mercurial, Jujutsu, Pijul, etc.) — anything that produces a directory with a `workspace.ncl`
- **Symlinks** to directories elsewhere on the filesystem

This design deliberately avoids coupling to git. A subworkspace is discovered by scanning for `workspace.ncl` files, not by parsing `.gitmodules` or any VCS metadata. As long as the directory exists at evaluation time and contains a `workspace.ncl`, it participates in the workspace.

Outputs are automatically namespaced:

```text
my-monorepo/
├── workspace.ncl              # Root workspace
├── packages/
│   └── shared-lib.ncl         # → packages.<system>.shared-lib
├── rust-nixos/                # Plain subdirectory
│   ├── workspace.ncl          # Subworkspace
│   └── packages/
│       └── default.ncl        # → packages.<system>.rust-nixos
├── mojo-zed/                  # Could be a git submodule
│   ├── workspace.ncl          # Subworkspace
│   └── packages/
│       └── default.ncl        # → packages.<system>.mojo-zed
│       └── lsp.ncl            # → packages.<system>.mojo-zed-lsp
└── backend/                   # Could be a jj/pijul checkout
    ├── workspace.ncl          # Subworkspace
    └── packages/
        └── default.ncl        # → packages.<system>.backend
        └── migrate.ncl        # → packages.<system>.backend-migrate
```

Namespacing rules:

- A subworkspace's `default` output uses the subworkspace directory name: `mojo-zed`
- Named outputs are prefixed with the subworkspace name: `mojo-zed-lsp`
- Root workspace outputs have no prefix
- All names are valid Nix derivation names (alphanumeric, hyphens, underscores)

Subworkspaces can declare dependencies on sibling subworkspaces:

```nickel
# mojo-zed/workspace.ncl
{
  name = "mojo-zed",
  dependencies = {
    wasm = "mojo-wasm",
  },
}
```

#### Git submodule example

A typical setup with git submodules:

```shell
# Add an external project as a subworkspace
git submodule add https://github.com/example/mojo-zed.git mojo-zed

# The submodule just needs a workspace.ncl to participate
ls mojo-zed/workspace.ncl  # ✓ discovered as a subworkspace
```

The root `flake.nix` does not need to change — `nix-workspace` discovers the subworkspace automatically at evaluation time. The submodule's `workspace.ncl` declares its own name, packages, and dependencies just like any other subworkspace.

#### Source considerations

When a subworkspace comes from a git submodule or external VCS, the Nix store path for its sources is determined by the flake's `self` input (which includes submodules when `git+file:.?submodules=1` is used). For non-git sources, the root flake may need to use `path:.` or a fetcher that captures the full directory tree. This is a Nix flake concern, not a `nix-workspace` concern — the workspace layer only requires that the paths resolve at evaluation time.

## Nickel contracts

### Contract hierarchy

```text
Workspace
├── WorkspaceConfig          # Top-level workspace.ncl structure
│   ├── mkWorkspaceConfig    # Factory for plugin-extended workspace contracts
│   └── DependencyRef        # Cross-subworkspace dependency reference (= Name)
├── NixpkgsConfig            # nixpkgs settings (allowUnfree, overlays, etc.)
├── ConventionConfig         # Per-convention directory overrides
│
├── PackageConfig            # Package definition
│   ├── RustPackage          # Rust-specific (plugin: nix-workspace-rust)
│   └── GoPackage            # Go-specific (plugin: nix-workspace-go)
│
├── ShellConfig              # Development shell
├── MachineConfig            # NixOS configuration
│   ├── StateVersion         # "YY.MM" pattern (e.g. "25.05")
│   ├── UserConfig           # Per-user home-manager config
│   ├── FileSystemConfig     # File system mount points
│   ├── NetworkingConfig     # Network settings
│   │   ├── FirewallConfig   # Firewall rules
│   │   └── InterfaceConfig  # Per-interface network config
│   ├── System               # "x86_64-linux" | "aarch64-linux" | ...
│   └── ModuleRef            # Reference to a NixOS module
│
├── HomeConfig               # Home-manager module definition
├── ModuleConfig             # NixOS module definition
│
├── OverlayConfig            # Nixpkgs overlay definition
├── CheckConfig              # CI check definition
├── TemplateConfig           # Flake template definition
│
├── PluginConfig             # Plugin definition contract
│   ├── PluginConvention     # Convention directory mapping
│   └── PluginBuilder        # Builder metadata + defaults
│
└── Common types             # Shared across all contracts
    ├── System               # Valid Nix system triple
    ├── Name                 # Valid workspace/output name
    ├── NonEmptyString       # Non-empty string
    ├── RelativePath         # Relative file path (no leading /)
    └── ModuleRef            # Module reference (name or path)
```

### Example contract

```nickel
# contracts/machine.ncl
{
  MachineConfig = {
    system
      | System
      | doc "Target system architecture",

    state-version
      | String
      | doc "NixOS state version (e.g. \"25.05\")"
      | default = "25.05",

    modules
      | Array ModuleRef
      | doc "NixOS modules to include"
      | default = [],

    special-args
      | { _ : Dyn }
      | doc "Extra arguments passed to modules"
      | default = {},
  },

  System = std.contract.from_predicate (fun s =>
    std.array.elem s [
      "x86_64-linux",
      "aarch64-linux",
      "x86_64-darwin",
      "aarch64-darwin",
    ]
  ),

  ModuleRef
    | doc "A reference to a NixOS module by name or path"
    = String,
}
```

### Error output

When a contract fails, the error includes:

```text
error: contract violation in machines/gravitas.ncl
  ┌─ machines/gravitas.ncl:3:13
  │
3 │   system = "x86-linux",
  │             ^^^^^^^^^^^ this value
  │
  expected: System (one of "x86_64-linux", "aarch64-linux", "x86_64-darwin", "aarch64-darwin")
       got: "x86-linux"
      hint: did you mean "x86_64-linux"?
  contract: MachineConfig.system
```

### Structured diagnostics

For programmatic consumption (AI agents, editors, CI), errors are also emitted as JSON:

```json
{
  "diagnostics": [
    {
      "code": "NW001",
      "severity": "error",
      "file": "machines/gravitas.ncl",
      "line": 3,
      "column": 13,
      "field": "system",
      "message": "Expected System (one of \"x86_64-linux\", \"aarch64-linux\", \"x86_64-darwin\", \"aarch64-darwin\"), got \"x86-linux\"",
      "hint": "Did you mean \"x86_64-linux\"?",
      "contract": "MachineConfig.system",
      "context": {
        "workspace": "my-project",
        "output": "nixosConfigurations.gravitas"
      }
    }
  ]
}
```

Diagnostic codes are prefixed `NW` (nix-workspace) and grouped:

- `NW0xx` — Contract violations (type/value errors)
- `NW1xx` — Discovery errors (missing files, bad directory structure)
- `NW2xx` — Namespace conflicts (duplicate names, invalid derivation names)
- `NW3xx` — Module errors (missing dependencies, circular imports)
- `NW4xx` — System/plugin errors (unsupported system, missing input, plugin conflicts)
- `NW5xx` — CLI errors (missing tool, tool failure, flake generation failure)

## System multiplexing

Systems are declared once and applied automatically:

```nickel
# workspace.ncl
{
  systems = ["x86_64-linux", "aarch64-linux"],

  packages = {
    my-tool = {
      # Available on all systems by default
      build-system = "rust",
    },
    linux-only = {
      # Override systems for specific packages
      systems = ["x86_64-linux"],
      build-system = "rust",
    },
  },
}
```

The Nix layer handles the `eachSystem` expansion. Users never write `packages.x86_64-linux.my-tool` — they write `packages.my-tool` and the system dimension is managed for them.

### Error catalog

| Code | Name | Description |
|------|------|-------------|
| `NW001` | Contract violation | A value failed a Nickel contract check |
| `NW002` | Invalid type | A value has the wrong type (e.g. string where array expected) |
| `NW003` | Invalid value | A value is the wrong shape or out of range |
| `NW100` | Missing workspace.ncl | No `workspace.ncl` found in the workspace root |
| `NW101` | Missing directory | A referenced convention directory does not exist |
| `NW102` | Invalid .ncl file | A `.ncl` file failed to parse or evaluate |
| `NW103` | Discovery error | General error during directory scanning |
| `NW200` | Namespace conflict | Two sources produce the same output name in the same convention |
| `NW201` | Invalid name | A namespaced output name is not a valid Nix derivation name |
| `NW300` | Missing dependency | A subworkspace declares a dependency on a non-existent sibling |
| `NW301` | Circular import | Circular dependency detected among subworkspaces |
| `NW400` | Unsupported system | A system string is not in the valid set / duplicate plugin name |
| `NW401` | Missing input | A required flake input is not provided |
| `NW402` | Plugin error | A plugin failed to load, validate, or merge |
| `NW500` | Missing tool | A required external tool (e.g. `nickel`, `nix`) is not on `$PATH` |
| `NW501` | Tool failed | An external tool invocation returned a non-zero exit code |
| `NW502` | Flake generation failed | On-the-fly `flake.nix` generation failed |

## Module system

`nix-workspace` supports plugins that extend the core functionality:

```nickel
# workspace.ncl
{
  plugins = [
    "nix-workspace-rust",      # Rust build support (Cargo integration)
    "nix-workspace-home",      # Home-manager integration
    "nix-workspace-deploy",    # Deployment support
  ],
}
```

### Plugin interface

A plugin is a Nickel record that can:

1. **Add contracts** — New configuration types (e.g., `RustPackage` with `cargo-features`, `edition`, etc.)
2. **Add conventions** — New directory mappings (e.g., `crates/` → Rust packages)
3. **Add builders** — Nix functions that construct outputs from config (e.g., `buildRustPackage`)
4. **Extend existing contracts** — Add fields to `PackageConfig`, `ShellConfig`, etc.

```nickel
# Plugin: nix-workspace-rust
{
  contracts = {
    RustPackage = {
      edition
        | [| '2021, '2024 |]
        | default = '2024,
      features
        | Array String
        | default = [],
      cargo-lock
        | String
        | default = "./Cargo.lock",
    },
  },

  conventions = {
    crates = {
      path = "crates",
      output = "packages",
      builder = "rust",
    },
  },
}
```

## Comparison

| Feature                    | nix-workspace          | flakelight                | flake-parts              |
|----------------------------|------------------------|---------------------------|--------------------------|
| Configuration language     | Nickel                 | Nix                       | Nix                      |
| Type validation            | Nickel contracts       | Nix module system types   | Nix module system types  |
| Error messages             | Structured + readable  | Nix stack traces          | Nix stack traces         |
| AI agent support           | JSON diagnostics       | None                      | None                     |
| Auto-discovery             | Convention directories | nixDir + aliases          | Manual                   |
| Subworkspaces              | Native                 | Manual imports            | Manual imports           |
| System multiplexing        | Automatic              | Automatic                 | perSystem module         |
| Plugin system              | Nickel plugins         | Flakelight modules        | flake-parts modules      |
| Standalone mode            | Planned                | No                        | No                       |

## Project structure

```text
nix-workspace/
├── SPEC.md                    # This specification
├── README.md                  # User-facing documentation
├── flake.nix                  # Flake definition (callable + dev tooling)
├── default.nix                # Package + dev shell for external consumers
├── justfile                   # Development task runner (build, test, run)
│
├── docs/                      # Documentation
│   ├── error-catalog.md       # Comprehensive NWxxx error catalog
│   ├── migration-guide.md     # Migration from flakelight / flake-parts
│   ├── editor-integration.md  # Editor LSP integration (Nickel LSP)
│   ├── ci-integration.md      # CI integration (GitHub Actions, GitLab CI, etc.)
│   └── stability.md           # Stability guarantees for contracts and APIs
│
├── lib/                       # Nix library (flake output builders)
│   ├── default.nix            # Main entry point: mkWorkspace function
│   ├── discover.nix           # Directory auto-discovery + subworkspace scanning
│   ├── eval-nickel.nix        # Nickel evaluation (wrapper generation, IFD bridge)
│   ├── systems.nix            # System multiplexing (eachSystem, perSystemOutput)
│   ├── namespacing.nix        # Subworkspace name resolution + conflict detection
│   ├── plugins.nix            # Plugin loading, routing, and merging
│   └── builders/              # Output-type-specific builders
│       ├── packages.nix
│       ├── shells.nix
│       ├── machines.nix
│       ├── modules.nix
│       ├── overlays.nix       # Overlay builder (v1.0)
│       ├── checks.nix         # Check builder (v1.0)
│       └── templates.nix      # Template builder (v1.0)
│
├── contracts/                 # Nickel contracts
│   ├── workspace.ncl          # Top-level workspace contract + mkWorkspaceConfig
│   ├── package.ncl            # Package contracts (PackageConfig)
│   ├── machine.ncl            # NixOS machine contracts (MachineConfig + sub-types)
│   ├── shell.ncl              # Dev shell contracts (ShellConfig)
│   ├── module.ncl             # Module contracts (ModuleConfig, HomeConfig)
│   ├── overlay.ncl            # Overlay contracts (OverlayConfig) (v1.0)
│   ├── check.ncl              # Check contracts (CheckConfig) (v1.0)
│   ├── template.ncl           # Template contracts (TemplateConfig) (v1.0)
│   ├── plugin.ncl             # Plugin definition contract (PluginConfig)
│   └── common.ncl             # Shared types (System, Name, ModuleRef, etc.)
│
├── plugins/                   # Built-in plugin definitions
│   ├── rust/                  # nix-workspace-rust plugin
│   │   ├── plugin.ncl         # Rust contracts, crates/ convention, extensions
│   │   └── builder.nix        # Rust builder (buildRustPackage wrapper)
│   └── go/                    # nix-workspace-go plugin
│       ├── plugin.ncl         # Go contracts, go-modules/ convention, extensions
│       └── builder.nix        # Go builder (buildGoModule wrapper)
│
├── cli/                       # Rust CLI (standalone mode)
│   ├── Cargo.toml
│   ├── Cargo.lock
│   └── src/
│       ├── main.rs            # Entry point, clap setup
│       ├── commands/
│       │   ├── mod.rs         # Subcommand enum + GlobalArgs
│       │   ├── init.rs        # `nix-workspace init`
│       │   ├── check.rs       # `nix-workspace check`
│       │   ├── info.rs        # `nix-workspace info`
│       │   ├── build.rs       # `nix-workspace build` (delegates to nix build)
│       │   └── shell.rs       # `nix-workspace shell` (delegates to nix develop)
│       ├── workspace.rs       # Workspace discovery (Rust-side directory scanning)
│       ├── nickel.rs          # Nickel evaluation wrapper (shells out to nickel CLI)
│       ├── diagnostics.rs     # Structured NW diagnostic types and formatting
│       └── flake_gen.rs       # On-the-fly flake.nix generation
│
├── examples/                  # Example workspaces
│   ├── minimal/               # Single package workspace
│   │   ├── flake.nix
│   │   ├── workspace.ncl
│   │   └── packages/
│   │       └── hello.ncl
│   ├── monorepo/              # Multi-subworkspace monorepo
│   │   ├── flake.nix
│   │   ├── workspace.ncl
│   │   ├── lib-a/
│   │   │   ├── workspace.ncl
│   │   │   └── packages/
│   │   │       └── default.ncl
│   │   └── app-b/
│   │       ├── workspace.ncl
│   │       └── packages/
│   │           └── default.ncl
│   ├── nixos/                 # NixOS machine configuration
│   │   ├── flake.nix
│   │   ├── workspace.ncl
│   │   ├── machines/
│   │   │   └── my-machine.ncl
│   │   └── modules/
│   │       └── desktop.ncl
│   ├── rust-project/          # Rust project with plugin support
│   │   ├── flake.nix
│   │   ├── workspace.ncl      # plugins = ["nix-workspace-rust"]
│   │   ├── packages/
│   │   │   └── my-cli.ncl     # Package with Rust plugin fields
│   │   └── crates/
│   │       └── my-lib.ncl     # Auto-discovered via crates/ convention
│   ├── submodule/             # Git submodule as subworkspace
│   │   ├── flake.nix
│   │   ├── workspace.ncl
│   │   └── external-tool/     # Simulated submodule with workspace.ncl
│   │       └── workspace.ncl
│   └── standalone/            # CLI standalone mode (no flake.nix)
│       ├── workspace.ncl
│       └── packages/
│           └── ...
│
└── tests/                     # Test suite
    ├── unit/                  # Nickel contract tests
    │   ├── workspace.ncl
    │   ├── package.ncl
    │   ├── machine.ncl
    │   ├── module.ncl
    │   ├── common.ncl         # Shared type contract tests
    │   ├── subworkspace.ncl   # Subworkspace contract tests
    │   ├── plugin.ncl         # Plugin contract tests
    │   ├── overlay.ncl        # Overlay contract tests (v1.0)
    │   ├── check.ncl          # Check contract tests (v1.0)
    │   └── template.ncl       # Template contract tests (v1.0)
    ├── integration/           # Full workspace evaluation tests (Nix)
    │   ├── discovery.nix
    │   ├── namespacing.nix
    │   └── plugins.nix        # Plugin system integration tests
    └── errors/                # Error message snapshot tests
        ├── invalid-system.ncl
        ├── invalid-machine-system.ncl
        ├── missing-machine-system.ncl
        ├── missing-field.ncl
        ├── invalid-build-system.ncl
        ├── invalid-state-version.ncl
        ├── invalid-plugin-name.ncl
        ├── invalid-plugin-convention.ncl
        ├── invalid-dependency-name.ncl
        ├── invalid-dependency-ref.ncl
        └── expected/          # Expected error output (JSON snapshots)
            ├── invalid-system.json
            ├── invalid-machine-system.json
            ├── missing-machine-system.json
            ├── missing-field.json
            ├── invalid-build-system.json
            ├── invalid-state-version.json
            ├── invalid-plugin-name.json
            ├── invalid-plugin-convention.json
            ├── invalid-dependency-name.json
            └── invalid-dependency-ref.json
```

## Non-goals

The following are explicitly out of scope for `nix-workspace`:

- **Replacing Nix** — `nix-workspace` is not a Nix alternative. It generates standard flake outputs; the Nix evaluator and store remain the execution engine.
- **Full Nickel-to-Nix bridge** — We do not depend on `nickel-nix` or attempt to evaluate Nix expressions from Nickel. The boundary is JSON: Nickel produces validated config, Nix consumes it.
- **Package manager** — `nix-workspace` does not fetch, resolve, or manage dependencies. Nixpkgs and flake inputs handle that.
- **Deployment orchestration** — While a `nix-workspace-deploy` plugin is conceivable, the core does not manage remote hosts, secrets, or rollout strategies.
- **Supporting non-flake Nix** — The Nix library assumes flakes. Legacy `nix-build` / `nix-shell` workflows are not targeted (though the CLI's standalone mode generates a flake on the fly).
- **VCS integration** — Subworkspace discovery is deliberately VCS-agnostic. We never parse `.gitmodules`, `.jj/`, or any VCS metadata.

## Security considerations

- **No network access at config time** — Nickel evaluation is pure and sandboxed. Network access only occurs during Nix fetching (controlled by flake inputs).
- **IFD boundary** — The Nickel evaluation step uses Import From Derivation (IFD) to bridge Nickel into Nix. This means Nickel evaluation happens in a Nix sandbox with the same restrictions as any other derivation.
- **Plugin trust** — Plugins ship Nix code (`builder.nix`) that runs with full Nix build privileges. Only load plugins from trusted sources. Built-in plugins (`nix-workspace-rust`, `nix-workspace-go`) are shipped with `nix-workspace` and reviewed as part of the project.
- **No secrets in config** — `workspace.ncl` files are committed to version control. Do not put secrets in Nickel config. Use NixOS modules (`sops-nix`, `agenix`) or environment variables for secret management.

## Versioning and stability

- **Spec version** — This specification tracks the design of `nix-workspace`. Breaking changes to the `workspace.ncl` schema or the Nix library API are reflected here.
- **Contract stability** — From v1.0 onward, core contracts (`WorkspaceConfig`, `PackageConfig`, `ShellConfig`, `MachineConfig`, `ModuleConfig`, `HomeConfig`) are stable. New optional fields may be added without a major version bump. Removing or renaming fields is a breaking change.
- **Plugin API stability** — The `PluginConfig` contract is stable from v1.0. Plugin authors can depend on the `contracts`, `conventions`, `builders`, and `extend` fields.
- **Diagnostic codes** — Once assigned, `NWxxx` codes are never reused or reassigned. Codes may be deprecated but not recycled.
- **Flake output shape** — `nix-workspace` always produces standard flake outputs. The output attribute structure is determined by Nix ecosystem conventions, not by `nix-workspace`.

## Milestones

### v0.1 — Foundation

- [x] Nickel contracts for core workspace config (`WorkspaceConfig`, `System`)
- [x] Nix library entry point (`nix-workspace` function callable from `flake.nix`)
- [x] Package auto-discovery from `packages/` directory
- [x] Dev shell auto-discovery from `shells/` directory
- [x] System multiplexing
- [x] Basic structured error output
- [x] Example: minimal workspace
- [x] Unit tests for contracts

### v0.2 — NixOS integration

- [x] `MachineConfig` contract with full validation (`StateVersion`, `UserConfig`, `FileSystemConfig`, `NetworkingConfig`)
- [x] `ModuleConfig` and `HomeConfig` contracts
- [x] Machine auto-discovery from `machines/` directory
- [x] Module auto-discovery from `modules/` directory
- [x] Home-manager module support (`home/` directory)
- [x] Example: NixOS machine configuration
- [x] Integration tests for machine building

### v0.3 — Subworkspaces

- [x] Subworkspace discovery and config merging
- [x] Automatic output namespacing with hyphen separator
- [x] Cross-subworkspace dependency resolution (`DependencyRef`, `validateDependencies`)
- [x] Cycle detection in dependency graph (`detectCycles`)
- [x] Namespace conflict detection with `NW2xx` diagnostics
- [x] Git submodule support (discover `workspace.ncl` in submodules)
- [x] VCS-agnostic discovery (any directory with `workspace.ncl`, not git-specific)
- [x] Example: monorepo with subworkspaces
- [x] Example: git submodule as subworkspace
- [x] Integration tests for namespacing

### v0.4 — Plugin system

- [x] Plugin interface definition (`PluginConfig`, `PluginConvention`, `PluginBuilder`)
- [x] Plugin loading and builder routing (`plugins.nix`)
- [x] Contract extension mechanism (`mkWorkspaceConfig` factory)
- [x] Built-in plugins: `nix-workspace-rust`, `nix-workspace-go`
- [x] Custom convention directory support (e.g. `crates/`, `go-modules/`)
- [x] Plugin validation and conflict detection (`NW4xx` diagnostics)
- [x] Shell extras from plugins (e.g. Rust toolchain in dev shells)
- [x] Plugin builder defaults applied to package configs

### v0.5 — Standalone CLI

- [x] Rust CLI skeleton (`nix-workspace init`, `check`, `info`, `build`, `shell`)
- [x] `nix-workspace build` delegating to `nix build`
- [x] `nix-workspace shell` delegating to `nix develop`
- [x] JSON diagnostic output via `--format json`
- [x] On-the-fly `flake.nix` generation for non-flake projects
- [x] Nickel error parsing into structured `NWxxx` diagnostics
- [x] Colored human-readable error output
- [x] Example: standalone workspace (no `flake.nix`)

### v1.0 — Production ready

- [x] Contract coverage for remaining flake output types (`OverlayConfig`, `CheckConfig`, `TemplateConfig`)
- [x] Comprehensive error catalog with all `NWxxx` codes documented and tested (`docs/error-catalog.md`)
- [x] Error snapshot tests with expected JSON output in `tests/errors/expected/`
- [x] Migration guide from flakelight / flake-parts (`docs/migration-guide.md`)
- [x] Editor integration (LSP diagnostics via Nickel LSP) (`docs/editor-integration.md`)
- [x] CI integration guide (GitHub Actions, etc.) (`docs/ci-integration.md`)
- [x] Full documentation and examples (`docs/`)
- [x] Stability guarantees for core contracts and plugin API (`docs/stability.md`)