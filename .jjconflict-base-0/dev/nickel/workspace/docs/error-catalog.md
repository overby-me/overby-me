# nix-workspace Error Catalog

This document describes every diagnostic code emitted by nix-workspace. Codes are prefixed `NW` and grouped by category. Once assigned, codes are never reused or reassigned — they may be deprecated but not recycled.

## Code Groups

| Range | Category | Source |
|-------|----------|--------|
| `NW0xx` | Contract violations | Nickel evaluation |
| `NW1xx` | Discovery errors | Nix discovery / CLI workspace scanning |
| `NW2xx` | Namespace conflicts | Subworkspace namespacing |
| `NW3xx` | Module/dependency errors | Dependency resolution, cycle detection |
| `NW4xx` | System/plugin errors | System multiplexing, plugin loading |
| `NW5xx` | CLI/tool errors | CLI infrastructure, external tool invocation |

---

## NW0xx — Contract Violations

These errors originate from Nickel contract evaluation. They indicate that a value in `workspace.ncl` (or a discovered `.ncl` file) does not satisfy its contract.

### NW001 — Contract violation

A value failed a Nickel contract check. This is the general-purpose code for contract failures that don't fall into a more specific category.

**Example trigger:**

```nickel
# workspace.ncl
{
  name = "my-project",
  packages = {
    my-tool = {
      build-inputs = "openssl",  # should be Array String, not String
    },
  },
}
```

**Typical message:**

```text
[NW001] Contract violation: expected Array String, got String
  field: packages.my-tool.build-inputs
  contract: PackageConfig.build-inputs
```

**Resolution:** Fix the value to match the expected type. Check the contract definition in `contracts/` for the expected shape.

---

### NW002 — Invalid type

A value has the wrong type — for example, a string where an array is expected, or a missing required field. This is a subclass of contract violations focused on structural/type mismatches.

**Example trigger:**

```nickel
# workspace.ncl — missing required `name` field
{
  description = "A workspace without a name",
  systems = ["x86_64-linux"],
}
```

**Typical message:**

```text
[NW002] missing required field "name" in WorkspaceConfig
  hint: Add a "name" field to your workspace.ncl, e.g.: name = "my-project"
```

**Resolution:** Add the missing required field or fix the type mismatch.

---

### NW003 — Invalid value

A value is the correct type but fails a semantic validation — for example, an unrecognized system string or an out-of-range number.

**Example trigger:**

```nickel
{
  name = "my-project",
  systems = ["x86-linux"],  # typo — should be "x86_64-linux"
}
```

**Typical message:**

```text
[NW003] invalid system "x86-linux" — did you mean "x86_64-linux"?
  hint: Valid systems: x86_64-linux, aarch64-linux, x86_64-darwin, aarch64-darwin
```

**Resolution:** Use one of the valid values listed in the hint.

**Other common NW003 triggers:**

- Invalid `build-system` value (must be `"rust"`, `"go"`, or `"generic"`)
- Invalid `state-version` format (must match `"YY.MM"`, e.g. `"25.05"`)
- Invalid `Name` pattern (must match `[a-zA-Z_][a-zA-Z0-9_-]*`)
- Negative overlay `priority` value

---

## NW1xx — Discovery Errors

These errors occur during the workspace scanning phase — when nix-workspace looks for `workspace.ncl`, convention directories, and `.ncl` files.

### NW100 — Missing workspace.ncl

No `workspace.ncl` file was found in the workspace root directory. This is the primary configuration file that every nix-workspace project requires.

**Example trigger:**

```bash
nix-workspace check
# (run in a directory without workspace.ncl)
```

**Typical message:**

```text
[NW100] No workspace.ncl found in /home/user/my-project
  hint: Run 'nix-workspace init' to create one, or create workspace.ncl manually.
```

**Resolution:** Create a `workspace.ncl` file or run `nix-workspace init`.

---

### NW101 — Missing directory

A referenced convention directory does not exist. This typically occurs when `workspace.ncl` overrides a convention path to a directory that hasn't been created yet.

**Example trigger:**

```nickel
{
  name = "my-project",
  conventions = {
    packages = { path = "src/packages" },
  },
}
# But src/packages/ does not exist
```

**Typical message:**

```text
[NW101] Convention directory 'src/packages' does not exist
  hint: Create the directory or remove the convention override.
```

**Resolution:** Create the referenced directory or remove the override.

---

### NW102 — Invalid .ncl file

A `.ncl` file in a convention directory failed to parse or evaluate. The file exists but contains syntax errors or evaluation failures.

**Example trigger:**

```nickel
# packages/broken.ncl
{
  build-system = "rust"
  # missing comma ↑
  description = "oops",
}
```

**Typical message:**

```text
[NW102] Failed to evaluate packages/broken.ncl
  file: packages/broken.ncl
  line: 3
  message: parse error: expected ',' or '}'
```

**Resolution:** Fix the syntax or evaluation error in the `.ncl` file.

---

### NW103 — Discovery error

A general error during directory scanning that doesn't fit the more specific NW10x codes. This can include filesystem permission errors, symlink resolution failures, etc.

**Typical message:**

```text
[NW103] Error scanning convention directory 'packages/': permission denied
```

**Resolution:** Check filesystem permissions and symlink targets.

---

## NW2xx — Namespace Conflicts

These errors occur during subworkspace output merging. When multiple sources produce outputs with the same name, nix-workspace detects and reports the conflict.

### NW200 — Namespace conflict

Two or more sources (root workspace, subworkspaces) produce the same output name in the same convention category.

**Example trigger:**

```text
my-project/
├── packages/
│   └── hello.ncl           → packages.hello
└── my-sub/
    ├── workspace.ncl        (name = "hello")
    └── packages/
        └── default.ncl      → packages.hello  ← CONFLICT!
```

**Typical message:**

```text
[NW200] Namespace conflict: 'hello' in 'packages' is produced by multiple sources: root, subworkspace:my-sub
  hint: Rename one of the conflicting outputs, or use a different subworkspace directory name.
```

**Resolution:** Rename one of the conflicting `.ncl` files, change the subworkspace directory name, or rename the subworkspace's `workspace.ncl` name.

---

### NW201 — Invalid name

A namespaced output name (produced by combining a subworkspace name with an output name) is not a valid Nix derivation name.

**Example trigger:**
A subworkspace directory named `123-bad` would produce output names like `123-bad` or `123-bad-foo`, which fail the `[a-zA-Z_][a-zA-Z0-9_-]*` pattern.

**Typical message:**

```text
[NW201] Invalid output name '123-bad' produced by subworkspace '123-bad'.
  Names must match [a-zA-Z_][a-zA-Z0-9_-]*.
  hint: Rename the subworkspace directory or the .ncl file to produce a valid name.
```

**Resolution:** Rename the subworkspace directory to start with a letter or underscore.

---

## NW3xx — Module/Dependency Errors

These errors relate to cross-subworkspace dependency resolution and module imports.

### NW300 — Missing dependency

A subworkspace declares a dependency on a sibling subworkspace that doesn't exist.

**Example trigger:**

```nickel
# lib-a/workspace.ncl
{
  name = "lib-a",
  dependencies = {
    utils = "lib-utils",  # no subworkspace named "lib-utils" exists
  },
}
```

**Typical message:**

```text
[NW300] Subworkspace 'lib-a' declares dependency 'utils' → 'lib-utils', but no subworkspace named 'lib-utils' exists.
  hint: Available subworkspaces: lib-b, app-c, shared
```

**Resolution:** Fix the dependency target name to match an existing subworkspace directory name.

---

### NW301 — Circular import

A circular dependency was detected among subworkspaces. Subworkspace A depends on B, which depends on A (or a longer cycle).

**Example trigger:**

```text
lib-a/workspace.ncl:  dependencies = { b = "lib-b" }
lib-b/workspace.ncl:  dependencies = { a = "lib-a" }
```

**Typical message:**

```text
[NW301] Circular dependency detected among subworkspaces: lib-a, lib-b
  hint: Break the cycle by removing one of the dependency declarations.
```

**Resolution:** Restructure dependencies to eliminate the cycle. Consider extracting shared code into a separate subworkspace.

---

## NW4xx — System/Plugin Errors

These errors relate to system multiplexing, flake input resolution, and plugin loading.

### NW400 — Unsupported system / Duplicate plugin

This code covers two related situations:

1. A system string is not in the valid set of supported systems.
2. A plugin is listed more than once in the `plugins` array.

**Example trigger (duplicate plugin):**

```nickel
{
  name = "my-project",
  plugins = ["nix-workspace-rust", "nix-workspace-rust"],
}
```

**Typical message:**

```text
[NW400] Plugin 'nix-workspace-rust' is listed 2 times in the plugins list.
  hint: Remove duplicate plugin entries from workspace.ncl.
```

**Resolution:** Remove the duplicate plugin entry.

---

### NW401 — Missing input

A required flake input is not provided. This occurs when `mkWorkspace` is called without necessary inputs (e.g., `nixpkgs`).

**Typical message:**

```text
[NW401] Required flake input 'nixpkgs' is not provided.
  hint: Make sure your flake.nix passes all required inputs to nix-workspace.
```

**Resolution:** Add the missing input to your `flake.nix` and pass it through to `nix-workspace`.

---

### NW402 — Plugin error

A plugin failed to load, validate, or merge. This covers issues like:

- Plugin `.ncl` file not found
- Plugin fails `PluginConfig` contract validation
- Plugin convention conflicts with another plugin
- Plugin builder not found

**Example trigger:**

```nickel
{
  name = "my-project",
  plugins = ["nix-workspace-nonexistent"],
}
```

**Typical message:**

```text
[NW402] Plugin 'nix-workspace-nonexistent' not found.
  hint: Available built-in plugins: nix-workspace-rust, nix-workspace-go
```

**Resolution:** Check the plugin name spelling and ensure the plugin is installed/available.

---

## NW5xx — CLI/Tool Errors

These errors are specific to the `nix-workspace` CLI and external tool invocations.

### NW500 — Missing tool

A required external tool (e.g., `nickel`, `nix`) is not found on `$PATH`.

**Example trigger:**

```bash
nix-workspace check
# (nickel binary not found)
```

**Typical message:**

```text
[NW500] Required tool 'nickel' not found on $PATH.
  hint: Install Nickel (https://nickel-lang.org/) or enter the nix-workspace dev shell.
```

**Resolution:** Install the missing tool or enter a development shell that provides it.

---

### NW501 — Tool failed

An external tool invocation (e.g., `nickel export`, `nix build`) returned a non-zero exit code.

**Example trigger:**

```bash
nix-workspace build my-tool
# nix build fails with an error
```

**Typical message:**

```text
[NW501] 'nix build' failed with exit code 1
  message: error: builder for '/nix/store/...-my-tool.drv' failed
```

**Resolution:** Check the underlying tool's error output for details.

---

### NW502 — Flake generation failed

On-the-fly `flake.nix` generation (used in standalone mode) failed. This occurs when the CLI cannot produce a valid `flake.nix` from the workspace configuration.

**Example trigger:**

```bash
nix-workspace build
# (in a directory without flake.nix, and generation fails)
```

**Typical message:**

```text
[NW502] Failed to generate flake.nix for standalone workspace.
  hint: Check that workspace.ncl is valid by running 'nix-workspace check'.
```

**Resolution:** Validate your `workspace.ncl` with `nix-workspace check`, or create a `flake.nix` manually.

---

## JSON Diagnostic Format

When using `--format json`, all diagnostics are emitted as structured JSON:

```json
{
  "diagnostics": [
    {
      "code": "NW003",
      "severity": "error",
      "file": "machines/gravitas.ncl",
      "line": 3,
      "column": 13,
      "field": "system",
      "message": "invalid system \"x86-linux\" — did you mean \"x86_64-linux\"?",
      "hint": "Valid systems: x86_64-linux, aarch64-linux, x86_64-darwin, aarch64-darwin",
      "contract": "MachineConfig.system",
      "context": {
        "workspace": "my-project",
        "output": "nixosConfigurations.gravitas"
      }
    }
  ]
}
```

### Field Reference

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `code` | string | yes | The `NWxxx` diagnostic code |
| `severity` | string | yes | One of `"error"`, `"warning"`, `"info"`, `"hint"` |
| `message` | string | yes | Human-readable error message |
| `file` | string | no | Source file path (relative to workspace root) |
| `line` | number | no | Line number in source file (1-based) |
| `column` | number | no | Column number in source file (1-based) |
| `field` | string | no | Dot-path to the offending field (e.g. `"packages.my-tool.build-system"`) |
| `hint` | string | no | Actionable suggestion for fixing the error |
| `contract` | string | no | The contract that was violated (e.g. `"PackageConfig.build-system"`) |
| `context` | object | no | Additional context about where the error occurred |
| `context.workspace` | string | no | The workspace name |
| `context.output` | string | no | The flake output path (e.g. `"nixosConfigurations.gravitas"`) |

### Severity Levels

| Severity | Meaning |
|----------|---------|
| `error` | The configuration is invalid and cannot be built. Must be fixed. |
| `warning` | The configuration is valid but may produce unexpected results. |
| `info` | Informational message (e.g., deprecation notice). |
| `hint` | Suggestion for improvement (non-blocking). |