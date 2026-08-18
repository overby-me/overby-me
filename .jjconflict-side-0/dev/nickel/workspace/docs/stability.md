# Stability Guarantees

This document describes the stability commitments for nix-workspace starting from v1.0. It covers core contracts, the plugin API, diagnostic codes, flake output shape, and the deprecation process.

## Table of Contents

- [Overview](#overview)
- [Stability tiers](#stability-tiers)
- [Core contracts](#core-contracts)
- [Plugin API](#plugin-api)
- [Diagnostic codes](#diagnostic-codes)
- [Flake output shape](#flake-output-shape)
- [CLI interface](#cli-interface)
- [Deprecation process](#deprecation-process)
- [What is NOT stable](#what-is-not-stable)
- [Version numbering](#version-numbering)

---

## Overview

nix-workspace follows [Semantic Versioning 2.0.0](https://semver.org/). From v1.0 onward:

- **Major version** (v2.0, v3.0) — Breaking changes to stable APIs.
- **Minor version** (v1.1, v1.2) — New features, new optional fields, new diagnostic codes. Backwards-compatible.
- **Patch version** (v1.0.1) — Bug fixes, documentation updates, performance improvements. No API changes.

The goal is to make `workspace.ncl` files written for v1.0 continue to work without modification on any v1.x release.

---

## Stability tiers

| Tier | Commitment | Examples |
|------|-----------|----------|
| **Stable** | No breaking changes within a major version. New optional fields may be added. | Core contracts, plugin API, diagnostic codes, CLI commands |
| **Provisional** | May change in minor versions with a deprecation period. | Experimental plugin features, new convention types |
| **Internal** | No stability guarantees. May change at any time. | Nix library internals, wrapper generation, IFD implementation |

---

## Core contracts

The following Nickel contracts are **stable** from v1.0:

| Contract | File | Stability |
|----------|------|-----------|
| `WorkspaceConfig` | `contracts/workspace.ncl` | Stable |
| `PackageConfig` | `contracts/package.ncl` | Stable |
| `ShellConfig` | `contracts/shell.ncl` | Stable |
| `MachineConfig` | `contracts/machine.ncl` | Stable |
| `ModuleConfig` | `contracts/module.ncl` | Stable |
| `HomeConfig` | `contracts/module.ncl` | Stable |
| `OverlayConfig` | `contracts/overlay.ncl` | Stable |
| `CheckConfig` | `contracts/check.ncl` | Stable |
| `TemplateConfig` | `contracts/template.ncl` | Stable |
| `NixpkgsConfig` | `contracts/workspace.ncl` | Stable |
| `ConventionConfig` | `contracts/workspace.ncl` | Stable |
| `DependencyRef` | `contracts/workspace.ncl` | Stable |

### Common types

| Type | File | Stability |
|------|------|-----------|
| `System` | `contracts/common.ncl` | Stable |
| `Name` | `contracts/common.ncl` | Stable |
| `NonEmptyString` | `contracts/common.ncl` | Stable |
| `RelativePath` | `contracts/common.ncl` | Stable |
| `ModuleRef` | `contracts/common.ncl` | Stable |

### Machine sub-types

| Type | File | Stability |
|------|------|-----------|
| `StateVersion` | `contracts/machine.ncl` | Stable |
| `UserConfig` | `contracts/machine.ncl` | Stable |
| `FileSystemConfig` | `contracts/machine.ncl` | Stable |
| `NetworkingConfig` | `contracts/machine.ncl` | Stable |
| `FirewallConfig` | `contracts/machine.ncl` | Stable |
| `InterfaceConfig` | `contracts/machine.ncl` | Stable |

### What "stable" means for contracts

1. **Existing fields are never removed or renamed.** A field present in v1.0 will be present and accepted in all v1.x releases.

2. **New optional fields may be added.** A v1.3 release might add a new optional field to `PackageConfig`. Existing `workspace.ncl` files that don't use this field continue to validate without changes.

3. **Default values are never changed.** If `build-system` defaults to `"generic"` in v1.0, it defaults to `"generic"` in all v1.x releases.

4. **Required fields are never added to existing contracts.** Adding a new required field would break existing configs. New required fields only appear in new contract types.

5. **Type constraints are never tightened.** If a field accepts `String` in v1.0, it will not be narrowed to `NonEmptyString` in v1.x. Constraints may be loosened (accept more values) in minor releases.

6. **Validation messages may change.** The exact wording of error messages, hints, and notes is not stable. Tooling should rely on diagnostic codes, not message text.

---

## Plugin API

The following plugin contracts are **stable** from v1.0:

| Contract | File | Stability |
|----------|------|-----------|
| `PluginConfig` | `contracts/plugin.ncl` | Stable |
| `PluginConvention` | `contracts/plugin.ncl` | Stable |
| `PluginBuilder` | `contracts/plugin.ncl` | Stable |

### Plugin API commitments

1. **The `PluginConfig` shape is stable.** Plugin authors can depend on the `contracts`, `conventions`, `builders`, and `extend` fields being present and working as documented.

2. **Plugin loading resolution is stable.** The `"nix-workspace-<shortname>"` → `plugins/<shortname>/` mapping is guaranteed.

3. **Plugin builder.nix interface is stable.** Plugins that export builder functions, `meta`, and `shellExtras` can depend on these being called as documented.

4. **New optional plugin features may be added.** For example, a future v1.x release might add a `hooks` field to `PluginConfig`. Existing plugins that don't use `hooks` continue to work.

5. **`mkWorkspaceConfig` is stable.** The factory function for building extended workspace contracts is a core part of the plugin system and will not change its signature.

### Built-in plugins

The built-in plugins (`nix-workspace-rust`, `nix-workspace-go`) are **stable** from v1.0. Their convention directories, builder names, and contract extensions will not change in breaking ways within a major version.

---

## Diagnostic codes

Diagnostic codes (`NWxxx`) follow strict rules:

1. **Codes are never reused or reassigned.** Once `NW200` means "Namespace conflict", it always means "Namespace conflict".

2. **Codes may be deprecated but not recycled.** A deprecated code continues to work but may emit a deprecation notice alongside the original diagnostic.

3. **New codes may be added in minor releases.** A v1.2 release might introduce `NW203` for a new kind of namespace issue.

4. **Code groupings are stable:**
   - `NW0xx` — Contract violations
   - `NW1xx` — Discovery errors
   - `NW2xx` — Namespace conflicts
   - `NW3xx` — Module/dependency errors
   - `NW4xx` — System/plugin errors
   - `NW5xx` — CLI/tool errors

5. **The JSON diagnostic format is stable.** The `{ diagnostics: [{ code, severity, message, ... }] }` structure will not change. New optional fields may be added to diagnostic records.

### Current code inventory

See [Error Catalog](./error-catalog.md) for the full list of assigned codes.

---

## Flake output shape

nix-workspace always produces **standard Nix flake outputs**. The output attribute structure follows Nix ecosystem conventions:

| Output | Shape | Stability |
|--------|-------|-----------|
| `packages.<system>.<name>` | Derivation | Stable |
| `devShells.<system>.<name>` | Derivation | Stable |
| `nixosConfigurations.<name>` | NixOS configuration | Stable |
| `nixosModules.<name>` | NixOS module | Stable |
| `homeModules.<name>` | Home-manager module | Stable |
| `overlays.<name>` | Overlay function | Stable |
| `checks.<system>.<name>` | Derivation | Stable |
| `templates.<name>` | Template attrset | Stable |
| `_pluginMeta` | Attrset (debug info) | Internal |

### Commitments

1. **nix-workspace never produces non-standard output attributes** (except `_pluginMeta`, which is prefixed with `_` to indicate it's internal).

2. **Output naming follows the workspace configuration.** The names in `workspace.ncl` map directly to flake output attribute names.

3. **System multiplexing follows the standard per-system pattern.** `packages.<system>.<name>`, not `packages.<name>.<system>`.

4. **Subworkspace namespacing is stable.** The `<subworkspace>-<name>` pattern (with `default` → `<subworkspace>`) will not change.

---

## CLI interface

The following CLI commands and flags are **stable** from v1.0:

### Commands

| Command | Description | Stability |
|---------|-------------|-----------|
| `nix-workspace init` | Initialize a new workspace | Stable |
| `nix-workspace check` | Validate workspace configuration | Stable |
| `nix-workspace info` | Show workspace structure | Stable |
| `nix-workspace build` | Build a package | Stable |
| `nix-workspace shell` | Enter a dev shell | Stable |

### Global flags

| Flag | Description | Stability |
|------|-------------|-----------|
| `--format human\|json` | Output format | Stable |
| `--workspace-dir DIR` | Override workspace root | Stable |
| `--version` | Show version | Stable |
| `--help` | Show help | Stable |

### Exit codes

| Code | Meaning | Stability |
|------|---------|-----------|
| `0` | Success | Stable |
| `1` | Validation errors or build failure | Stable |
| `2` | Infrastructure error | Stable |

### Commitments

1. **Existing commands are never removed.** New subcommands may be added.
2. **Existing flags are never removed or renamed.** New flags may be added.
3. **Exit codes are stable.** The meaning of exit codes 0, 1, and 2 will not change.
4. **JSON output format is stable.** The structure of `--format json` output follows the diagnostic format described above.

---

## Deprecation process

When a feature needs to change in a breaking way, we follow this process:

### For minor releases (v1.x)

1. **Announce deprecation** in release notes.
2. **Emit a warning** (`severity: "warning"`) when the deprecated feature is used, with a `hint` explaining the migration path.
3. **Maintain the deprecated feature** for at least two minor releases.
4. **Document the migration path** in release notes and the migration guide.

### For major releases (v2.0)

1. **Remove deprecated features** that have been deprecated for at least one full minor release cycle.
2. **Provide a migration guide** from v1.x to v2.0.
3. **Update all contracts** to reflect the new shape.

### Example deprecation timeline

```text
v1.2 — Field `foo` is deprecated. Warning emitted when used.
        Hint: "Use 'bar' instead of 'foo'. 'foo' will be removed in v2.0."
v1.3 — Field `foo` still works but warns.
v2.0 — Field `foo` is removed.
```

---

## What is NOT stable

The following are explicitly **internal** and may change without notice:

1. **Nix library internals** — The implementation of `lib/default.nix`, `lib/eval-nickel.nix`, `lib/discover.nix`, etc. These are internal to nix-workspace and not part of the public API.

2. **Nickel wrapper generation** — The generated Nickel wrapper code that bridges workspace.ncl to JSON evaluation. The wrapper format may change between any releases.

3. **IFD implementation details** — How Import From Derivation is used to evaluate Nickel. The derivation structure, builder scripts, and temporary file layout may change.

4. **Error message wording** — The exact text of error messages, hints, and notes. Tooling should use diagnostic codes, not string matching.

5. **`_pluginMeta` output** — The debug metadata exposed when plugins are loaded. This is for development introspection only.

6. **Internal builder functions** — The specific Nix functions in `lib/builders/*.nix`. These are called by `mkWorkspace` internally and are not part of the public API.

7. **Test infrastructure** — Test files, test helpers, and test structure may change freely.

8. **Convention directory implementation** — While the convention directory names (`packages/`, `shells/`, etc.) are stable, the internal discovery implementation may change.

---

## Version numbering

### Current version

nix-workspace v1.0 is the first stable release. All contracts, APIs, and conventions described in this document are frozen for the v1.x series.

### Pre-1.0 versions

Versions before v1.0 (v0.1 through v0.5) were development milestones with no stability guarantees. Breaking changes occurred freely between milestones.

### Checking your version

```bash
# CLI version
nix-workspace --version

# Flake version (from flake metadata)
nix flake metadata github:example/nix-workspace
```

### Compatibility matrix

| nix-workspace | Nickel | Nix | NixOS |
|---------------|--------|-----|-------|
| v1.0 | >= 1.5 | >= 2.18 | 24.11+ |

These are minimum supported versions. nix-workspace is tested against the latest stable releases of each dependency.