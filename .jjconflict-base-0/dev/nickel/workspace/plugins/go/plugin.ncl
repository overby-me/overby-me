# nix-workspace-go — Built-in Go plugin
#
# Provides Go module build support for nix-workspace:
#   - GoPackage contract with Go-specific fields
#   - go-modules/ convention directory mapping
#   - Enhanced Go builder with module/vendor support
#
# Usage in workspace.ncl:
#   { plugins = ["nix-workspace-go"] }
#
# Then either:
#   1. Place .ncl files in go-modules/ (auto-discovered via convention)
#   2. Use build-system = "go" in packages/ .ncl files
#   3. Use the GoPackage contract fields on any Go package config
{
  name = "nix-workspace-go",
  description = "Go module build support for nix-workspace",
  version = "0.4.0",

  # ── Contracts ─────────────────────────────────────────────────
  #
  # GoPackage defines Go-specific configuration fields. These are
  # validated by Nickel before reaching the Nix builder.
  contracts = {
    GoPackage = {
      go-version
        | std.contract.from_validator (fun value =>
          if !(std.is_string value) then
            'Error { message = "expected a string, got %{std.typeof value}" }
          else if std.array.elem value ["1_21", "1_22", "1_23", "1_24"] then
            'Ok
          else
            'Error {
              message = "invalid Go version \"%{value}\"",
              notes = ["Valid versions: \"1_21\", \"1_22\", \"1_23\", \"1_24\""],
            }
        )
        | doc m%"
            Go toolchain version to use for building.
            Maps to the corresponding Go package in nixpkgs
            (e.g. "1_23" → pkgs.go_1_23).
          "%
        | default
        = "1_23",

      vendor-hash
        | String
        | doc m%"
            The hash of the vendored dependencies.
            Set to "" (empty string) for a first build to get the correct hash
            from the error message, or use null (via "lib.fakeHash") for
            packages that vendor their dependencies in-tree.
          "%
        | optional,

      sub-packages
        | Array String
        | doc m%"
            Go sub-packages to build, relative to the module root.
            If empty, the default package (.) is built.
            Example: ["./cmd/mytool", "./cmd/myserver"]
          "%
        | default
        = [],

      tags
        | Array String
        | doc m%"
            Go build tags to pass via -tags flag.
            Example: ["netgo", "osusergo"]
          "%
        | default
        = [],

      ldflags
        | Array String
        | doc m%"
            Linker flags passed via -ldflags.
            Example: ["-s", "-w", "-X main.version=1.0.0"]
          "%
        | default
        = [],

      cgo-enabled
        | Bool
        | doc "Whether to enable CGO during the build"
        | default
        = false,

      proxy-vendor
        | Bool
        | doc m%"
            Whether to use Go module proxy for fetching dependencies.
            When false, dependencies are fetched directly.
          "%
        | default
        = true,

      check-flags
        | Array String
        | doc m%"
            Extra flags passed to 'go test' during the check phase.
            Example: ["-race", "-count=1"]
          "%
        | default
        = [],

      do-check
        | Bool
        | doc "Whether to run tests during the build"
        | default
        = true,
    },
  },

  # ── Conventions ─────────────────────────────────────────────────
  #
  # The go-modules/ directory is scanned for .ncl files, each producing
  # a package output built with the "go" builder.
  #
  # Example:
  #   go-modules/
  #   ├── my-api.ncl       → packages.<system>.my-api
  #   └── my-cli.ncl       → packages.<system>.my-cli
  conventions = {
    go-modules = {
      path = "go-modules",
      output = "packages",
      builder = "go",
      auto-discover = true,
    },
  },

  # ── Builders ────────────────────────────────────────────────────
  #
  # Metadata for the Go builder. The actual Nix build logic is in
  # plugins/go/builder.nix — this record provides defaults that are
  # merged into package configs routed to the "go" builder.
  builders = {
    go = {
      name = "go",
      description = "Build Go modules via buildGoModule",
      defaults = {
        build-system = "go",
      },
    },
  },

  # ── Contract extensions ─────────────────────────────────────────
  #
  # Extend the core PackageConfig with Go-specific optional fields.
  # These fields are only meaningful for packages with build-system = "go"
  # but are always allowed (as optional) to avoid contract violations
  # when a workspace loads the Go plugin.
  extend = {
    PackageConfig = {
      go-version
        | std.contract.from_validator (fun value =>
          if !(std.is_string value) then
            'Error { message = "expected a string, got %{std.typeof value}" }
          else if std.array.elem value ["1_21", "1_22", "1_23", "1_24"] then
            'Ok
          else
            'Error {
              message = "invalid Go version \"%{value}\"",
              notes = ["Valid versions: \"1_21\", \"1_22\", \"1_23\", \"1_24\""],
            }
        )
        | doc "Go toolchain version (requires nix-workspace-go plugin)"
        | optional,

      vendor-hash
        | String
        | doc "Hash of vendored Go dependencies (requires nix-workspace-go plugin)"
        | optional,

      sub-packages
        | Array String
        | doc "Go sub-packages to build (requires nix-workspace-go plugin)"
        | optional,

      tags
        | Array String
        | doc "Go build tags (requires nix-workspace-go plugin)"
        | optional,

      ldflags
        | Array String
        | doc "Go linker flags (requires nix-workspace-go plugin)"
        | optional,

      cgo-enabled
        | Bool
        | doc "Enable CGO (requires nix-workspace-go plugin)"
        | optional,

      proxy-vendor
        | Bool
        | doc "Use Go module proxy (requires nix-workspace-go plugin)"
        | optional,

      check-flags
        | Array String
        | doc "Extra go test flags (requires nix-workspace-go plugin)"
        | optional,

      do-check
        | Bool
        | doc "Run tests during build (requires nix-workspace-go plugin)"
        | optional,
    },
  },
}
