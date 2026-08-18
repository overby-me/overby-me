# CI Integration Guide

This guide shows how to integrate nix-workspace into your continuous integration pipeline. nix-workspace's structured diagnostic output (`--format json`) makes it particularly well-suited for CI environments where machine-readable error reporting is essential.

## Table of Contents

- [Overview](#overview)
- [GitHub Actions](#github-actions)
  - [Basic workflow](#basic-workflow)
  - [Matrix builds (multiple systems)](#matrix-builds-multiple-systems)
  - [Caching](#caching)
  - [Pull request annotations](#pull-request-annotations)
- [GitLab CI](#gitlab-ci)
- [Generic CI (any platform)](#generic-ci-any-platform)
- [JSON diagnostics in CI](#json-diagnostics-in-ci)
- [Pre-commit hooks](#pre-commit-hooks)
- [Best practices](#best-practices)
- [Troubleshooting](#troubleshooting)

---

## Overview

A typical nix-workspace CI pipeline has three stages:

1. **Check** — Validate `workspace.ncl` and all discovered `.ncl` files against contracts.
2. **Build** — Build all packages for the target system(s).
3. **Test** — Run `nix flake check` to execute checks, test suites, and contract tests.

All three stages produce structured diagnostics when using `--format json`, which can be parsed by CI tooling for inline annotations, error summaries, and failure categorization.

---

## GitHub Actions

### Basic workflow

```yaml
# .github/workflows/nix-workspace.yml
name: nix-workspace CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: DeterminateSystems/nix-installer-action@main

      - uses: DeterminateSystems/magic-nix-cache-action@main

      - name: Validate workspace configuration
        run: |
          nix develop --command nix-workspace check --format json > diagnostics.json 2>&1 || true
          # Display human-readable output
          nix develop --command nix-workspace check

      - name: Build all packages
        run: nix build

      - name: Run flake checks
        run: nix flake check

      - name: Upload diagnostics
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: diagnostics
          path: diagnostics.json
```

### Matrix builds (multiple systems)

To test across multiple architectures using GitHub's hosted runners and QEMU:

```yaml
name: Multi-system CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  build:
    strategy:
      matrix:
        include:
          - system: x86_64-linux
            runner: ubuntu-latest
          - system: aarch64-linux
            runner: ubuntu-latest
          # macOS runners for Darwin systems (if needed):
          # - system: x86_64-darwin
          #   runner: macos-13
          # - system: aarch64-darwin
          #   runner: macos-14

    runs-on: ${{ matrix.runner }}

    steps:
      - uses: actions/checkout@v4

      - uses: DeterminateSystems/nix-installer-action@main
        with:
          extra-conf: |
            extra-platforms = aarch64-linux

      - uses: DeterminateSystems/magic-nix-cache-action@main

      # For aarch64-linux on x86_64 runners, enable QEMU
      - name: Set up QEMU
        if: matrix.system == 'aarch64-linux'
        uses: docker/setup-qemu-action@v3
        with:
          platforms: arm64

      - name: Validate configuration
        run: nix develop --command nix-workspace check

      - name: Build packages
        run: nix build --system ${{ matrix.system }}

      - name: Run checks
        run: nix flake check --system ${{ matrix.system }}
```

### Caching

The [Magic Nix Cache](https://github.com/DeterminateSystems/magic-nix-cache-action) action (shown above) is the easiest way to cache Nix store paths in GitHub Actions. It works automatically with no configuration.

For self-hosted caches (e.g., [Cachix](https://cachix.org/)):

```yaml
      - uses: cachix/install-nix-action@v27
        with:
          nix_path: nixpkgs=channel:nixos-unstable

      - uses: cachix/cachix-action@v15
        with:
          name: my-cache
          authToken: '${{ secrets.CACHIX_AUTH_TOKEN }}'
```

### Pull request annotations

Parse JSON diagnostics and convert them to GitHub Actions annotations for inline PR feedback:

```yaml
      - name: Check with annotations
        run: |
          set +e
          nix develop --command nix-workspace check --format json > /tmp/diagnostics.json 2>&1
          exit_code=$?
          set -e

          # Parse diagnostics and emit GitHub annotations
          if [ -f /tmp/diagnostics.json ]; then
            python3 - << 'PYTHON'
          import json, sys

          try:
              with open("/tmp/diagnostics.json") as f:
                  report = json.load(f)
          except (json.JSONDecodeError, FileNotFoundError):
              sys.exit(0)

          for d in report.get("diagnostics", []):
              severity = d.get("severity", "error")
              # Map nix-workspace severity to GitHub annotation level
              level = {"error": "error", "warning": "warning", "info": "notice"}.get(severity, "error")

              msg = f"[{d.get('code', '?')}] {d.get('message', 'unknown error')}"
              if d.get("hint"):
                  msg += f"\nhint: {d['hint']}"

              file_part = ""
              if d.get("file"):
                  file_part = f"file={d['file']}"
                  if d.get("line"):
                      file_part += f",line={d['line']}"
                  if d.get("column"):
                      file_part += f",col={d['column']}"

              if file_part:
                  print(f"::{level} {file_part}::{msg}")
              else:
                  print(f"::{level}::{msg}")
          PYTHON
          fi

          exit $exit_code
```

---

## GitLab CI

```yaml
# .gitlab-ci.yml
stages:
  - validate
  - build
  - test

variables:
  NIX_CONFIG: "experimental-features = nix-command flakes"

.nix-base:
  image: nixos/nix:latest
  before_script:
    - nix --version

validate:
  extends: .nix-base
  stage: validate
  script:
    - nix develop --command nix-workspace check --format json | tee diagnostics.json
    - nix develop --command nix-workspace check
  artifacts:
    when: always
    paths:
      - diagnostics.json
    reports:
      # GitLab can parse codequality reports
      codequality: diagnostics-codequality.json
  after_script:
    # Convert nix-workspace JSON to GitLab Code Quality format
    - |
      python3 -c "
      import json
      try:
          with open('diagnostics.json') as f:
              report = json.load(f)
      except:
          exit(0)

      issues = []
      for d in report.get('diagnostics', []):
          severity_map = {'error': 'critical', 'warning': 'major', 'info': 'minor'}
          issues.append({
              'type': 'issue',
              'check_name': d.get('code', 'NW000'),
              'description': d.get('message', ''),
              'severity': severity_map.get(d.get('severity'), 'major'),
              'location': {
                  'path': d.get('file', 'workspace.ncl'),
                  'lines': {'begin': d.get('line', 1)},
              },
          })

      with open('diagnostics-codequality.json', 'w') as f:
          json.dump(issues, f)
      "

build:
  extends: .nix-base
  stage: build
  script:
    - nix build
  artifacts:
    paths:
      - result

test:
  extends: .nix-base
  stage: test
  script:
    - nix flake check
```

---

## Generic CI (any platform)

For CI systems without specific integrations (Jenkins, Buildkite, CircleCI, etc.), the core commands are:

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "==> Step 1: Validate workspace configuration"
nix develop --command nix-workspace check --format json > diagnostics.json
nix develop --command nix-workspace check

echo "==> Step 2: Show workspace structure"
nix develop --command nix-workspace info

echo "==> Step 3: Build all packages"
nix build

echo "==> Step 4: Run flake checks"
nix flake check

echo "==> All CI steps passed."
```

### Exit codes

| Exit code | Meaning |
|-----------|---------|
| `0` | Success — all validations passed |
| `1` | Validation errors or build failure |
| `2` | Infrastructure error (missing tool, bad arguments) |

---

## JSON diagnostics in CI

The `--format json` flag produces structured output that's ideal for CI:

```bash
# Capture diagnostics as JSON
nix-workspace check --format json > diagnostics.json

# Count errors
error_count=$(jq '[.diagnostics[] | select(.severity == "error")] | length' diagnostics.json)
echo "Found $error_count error(s)"

# Extract error codes for categorization
jq -r '.diagnostics[] | .code' diagnostics.json | sort | uniq -c | sort -rn

# Get all messages for a summary
jq -r '.diagnostics[] | "[" + .code + "] " + .message' diagnostics.json
```

### Example JSON output

```json
{
  "diagnostics": [
    {
      "code": "NW003",
      "severity": "error",
      "file": "packages/my-tool.ncl",
      "line": 5,
      "field": "build-system",
      "message": "unknown build-system \"python\"",
      "hint": "Supported build systems: \"rust\", \"go\", \"generic\".",
      "contract": "PackageConfig.build-system"
    }
  ]
}
```

### Failing the build on warnings

By default, only errors cause a non-zero exit code. To also fail on warnings:

```bash
nix-workspace check --format json > diagnostics.json

warning_count=$(jq '[.diagnostics[] | select(.severity == "warning")] | length' diagnostics.json)
error_count=$(jq '[.diagnostics[] | select(.severity == "error")] | length' diagnostics.json)

if [ "$error_count" -gt 0 ] || [ "$warning_count" -gt 0 ]; then
  echo "CI failed: $error_count error(s), $warning_count warning(s)"
  jq -r '.diagnostics[] | "[" + .severity + "] [" + .code + "] " + .message' diagnostics.json
  exit 1
fi
```

---

## Pre-commit hooks

### With pre-commit framework

```yaml
# .pre-commit-config.yaml
repos:
  - repo: local
    hooks:
      - id: nix-workspace-check
        name: Validate nix-workspace config
        entry: nix-workspace check
        language: system
        pass_filenames: false
        files: '\.ncl$'

      - id: nickel-format
        name: Format Nickel files
        entry: nickel format
        language: system
        types: [file]
        files: '\.ncl$'
```

### With nix-workspace's git hooks

If your project uses devenv or a custom git hooks setup:

```bash
#!/usr/bin/env bash
# .git/hooks/pre-commit (or via your hooks framework)

# Only run if .ncl files were modified
if git diff --cached --name-only | grep -q '\.ncl$'; then
  echo "Validating workspace configuration..."
  if ! nix-workspace check; then
    echo ""
    echo "Workspace validation failed. Fix the errors above before committing."
    echo "Run 'nix-workspace check --format json' for structured output."
    exit 1
  fi
fi
```

---

## Best practices

### 1. Run `check` before `build`

The `nix-workspace check` command validates configuration in seconds using Nickel evaluation alone — no Nix builds needed. Always run it first to catch typos and contract violations early:

```yaml
- name: Quick validation
  run: nix-workspace check  # Fast: Nickel-only

- name: Full build
  run: nix build  # Slow: full Nix evaluation + building
```

### 2. Cache aggressively

Nix builds are deterministic and cacheable. Use a binary cache (Cachix, Attic, or GitHub Actions cache) to avoid rebuilding unchanged derivations.

### 3. Use JSON output for automation

Always capture `--format json` output as a CI artifact. Even if the build succeeds, diagnostics may contain warnings worth reviewing.

### 4. Pin nix-workspace version

In `flake.nix`, consider pinning to a specific version or commit of nix-workspace for reproducible CI:

```nix
inputs = {
  nix-workspace.url = "github:example/nix-workspace/v1.0.0";
};
```

### 5. Validate on `.ncl` file changes

Configure your CI to run validation whenever `.ncl` files change, even in PRs that don't modify Nix files:

```yaml
on:
  pull_request:
    paths:
      - '**/*.ncl'
      - 'flake.nix'
      - 'flake.lock'
```

### 6. Separate check and build jobs

Keep validation and building as separate CI jobs so that fast feedback (contract violations) isn't blocked by slow builds:

```yaml
jobs:
  validate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: DeterminateSystems/nix-installer-action@main
      - run: nix develop --command nix-workspace check

  build:
    needs: validate
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: DeterminateSystems/nix-installer-action@main
      - uses: DeterminateSystems/magic-nix-cache-action@main
      - run: nix build
      - run: nix flake check
```

---

## Troubleshooting

### `nix-workspace` command not found

The CLI is available inside the nix-workspace dev shell. In CI, use:

```bash
nix develop --command nix-workspace check
```

Or install it globally:

```bash
nix profile install github:example/nix-workspace
```

### IFD (Import From Derivation) failures

nix-workspace uses IFD to bridge Nickel evaluation into Nix. Some CI environments restrict IFD. Ensure your Nix configuration allows it:

```text
# nix.conf or NIX_CONFIG
allow-import-from-derivation = true
```

### Sandbox issues

If Nickel evaluation fails in a sandboxed build, ensure the sandbox has access to the Nickel binary. The `bootstrapPkgs` in nix-workspace handles this automatically, but custom sandbox configurations may need adjustment.

### `nickel` binary not found during Nix evaluation

This usually means the IFD derivation can't find Nickel. This is handled automatically by nix-workspace's `eval-nickel.nix`, but if you're using a non-standard Nix setup, ensure `nixpkgs` is available in your flake inputs.

### Timeouts

Nickel contract evaluation is fast (typically < 1 second), but IFD adds overhead because Nix must build the evaluation derivation. For large workspaces:

1. Ensure binary caches are configured — the Nickel evaluation derivation should be cached after the first build.
2. Increase CI timeout for the first run (subsequent runs use cached results).
3. Consider running `nix-workspace check` (Nickel-only, no IFD) as a fast pre-check before `nix build`.