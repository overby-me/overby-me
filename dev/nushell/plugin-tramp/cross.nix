# Cross-compilation matrix for tramp-agent binaries.
#
# This module provides derivations for building the tramp-agent binary
# for multiple target architectures from a single host.  The resulting
# binaries are statically linked (via musl) for maximum portability —
# they can be deployed to any Linux host regardless of its libc.
#
# ## Usage
#
# ```nix
# # In your flake.nix:
# {
#   outputs = { self, nixpkgs, ... }: let
#     cross = import ./nu-plugin-tramp/cross.nix { inherit nixpkgs; };
#   in {
#     packages.x86_64-linux = {
#       tramp-agent-x86_64-linux = cross.x86_64-linux;
#       tramp-agent-aarch64-linux = cross.aarch64-linux;
#     };
#   };
# }
# ```
#
# ## Supported targets
#
# | Target              | Triple                        | Notes                    |
# |---------------------|-------------------------------|--------------------------|
# | x86_64-linux        | x86_64-unknown-linux-musl     | Most servers & desktops  |
# | aarch64-linux       | aarch64-unknown-linux-musl    | ARM64 (Raspberry Pi 4+) |
# | x86_64-darwin       | x86_64-apple-darwin           | Intel Mac                |
# | aarch64-darwin      | aarch64-apple-darwin          | Apple Silicon Mac        |
#
# Linux targets use musl for static linking.  Darwin targets use the
# native Apple SDK (cross-compilation from Linux to macOS requires
# additional toolchain setup and is not included here — build natively
# on macOS instead).
{
  nixpkgs ? null,
  lib ? nixpkgs.lib or (import <nixpkgs> {}).lib,
}: let
  # Source filtering — only include Cargo files and the crates directory.
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./crates
    ];
  };

  # Common build attributes shared across all targets.
  commonAttrs = {
    pname = "tramp-agent";
    version = "unstable";

    inherit src;

    cargoLock.lockFile = ./Cargo.lock;

    cargoBuildFlags = ["-p" "tramp-agent"];
    cargoTestFlags = ["-p" "tramp-agent"];

    # Use the size-optimised release profile for the agent binary.
    CARGO_PROFILE = "release-agent";

    # Skip tests during cross-compilation (they need to run on the target).
    doCheck = false;

    meta = {
      description = "Lightweight RPC agent for nushell-plugin-tramp (cross-compiled)";
      homepage = "https://tangled.org/overby.me/overby.me/tree/main/nushell/plugin-tramp";
      license = lib.licenses.mit;
      mainProgram = "tramp-agent";
    };
  };

  # Build a tramp-agent for a specific pkgs set.
  buildAgent = pkgs:
    pkgs.rustPlatform.buildRustPackage (commonAttrs
      // {
        # Ensure the binary is stripped for minimal size.
        stripAllList = ["bin"];
      });

  # Build a statically-linked tramp-agent using musl (Linux targets only).
  buildAgentStatic = pkgs:
    pkgs.rustPlatform.buildRustPackage (commonAttrs
      // {
        # Static linking flags for musl targets.
        CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";

        stripAllList = ["bin"];
      });

  # Helper to get cross-compilation pkgs for a target system.
  #
  # For Linux musl targets, we use pkgsStatic which sets up the full
  # musl cross-compilation toolchain.
  #
  # For Darwin targets, we use the native pkgs for that system (cross
  # from Linux to macOS is not practical without the Apple SDK).
  getCrossPkgs = buildSystem: targetSystem:
    if lib.match ".*-linux" targetSystem != null
    then
      # Use pkgsStatic for musl-based static builds.
      (import nixpkgs {
        system = buildSystem;
        crossSystem = {
          config =
            if targetSystem == "x86_64-linux"
            then "x86_64-unknown-linux-musl"
            else if targetSystem == "aarch64-linux"
            then "aarch64-unknown-linux-musl"
            else throw "Unsupported Linux target: ${targetSystem}";
          isStatic = true;
        };
      })
    else
      # Darwin targets — build natively (no cross from Linux).
      (import nixpkgs {system = targetSystem;});
in {
  # --------------------------------------------------------------------------
  # Per-target derivations (built from x86_64-linux by default)
  # --------------------------------------------------------------------------

  # Build for x86_64 Linux (statically linked with musl).
  x86_64-linux = buildAgentStatic (getCrossPkgs "x86_64-linux" "x86_64-linux");

  # Build for aarch64 Linux (statically linked with musl, cross-compiled).
  aarch64-linux = buildAgentStatic (getCrossPkgs "x86_64-linux" "aarch64-linux");

  # Build for x86_64 macOS (must be built on an Intel Mac or macOS runner).
  x86_64-darwin = buildAgent (getCrossPkgs "x86_64-darwin" "x86_64-darwin");

  # Build for aarch64 macOS (must be built on Apple Silicon or macOS runner).
  aarch64-darwin = buildAgent (getCrossPkgs "aarch64-darwin" "aarch64-darwin");

  # --------------------------------------------------------------------------
  # Convenience: all Linux targets from a given build system
  # --------------------------------------------------------------------------

  # Build all Linux agent binaries from a specific host system.
  #
  # Usage: `(import ./cross.nix { inherit nixpkgs; }).allLinuxFrom "x86_64-linux"`
  allLinuxFrom = buildSystem: {
    x86_64-linux = buildAgentStatic (getCrossPkgs buildSystem "x86_64-linux");
    aarch64-linux = buildAgentStatic (getCrossPkgs buildSystem "aarch64-linux");
  };

  # --------------------------------------------------------------------------
  # Convenience: build matrix for CI
  # --------------------------------------------------------------------------

  # A matrix of all targets, suitable for iterating in CI.
  #
  # Each entry has:
  #   - `triple`: the Rust target triple
  #   - `system`: the Nix system string
  #   - `buildSystem`: the recommended build host
  #   - `static`: whether the binary is statically linked
  matrix = [
    {
      triple = "x86_64-unknown-linux-musl";
      system = "x86_64-linux";
      buildSystem = "x86_64-linux";
      static' = true;
    }
    {
      triple = "aarch64-unknown-linux-musl";
      system = "aarch64-linux";
      buildSystem = "x86_64-linux";
      static' = true;
    }
    {
      triple = "x86_64-apple-darwin";
      system = "x86_64-darwin";
      buildSystem = "x86_64-darwin";
      static' = false;
    }
    {
      triple = "aarch64-apple-darwin";
      system = "aarch64-darwin";
      buildSystem = "aarch64-darwin";
      static' = false;
    }
  ];

  # --------------------------------------------------------------------------
  # Helper: cache directory structure
  # --------------------------------------------------------------------------

  # Build a derivation that produces the local cache directory structure
  # expected by the plugin's deployment module:
  #
  #   $out/
  #   ├── x86_64-unknown-linux-musl/
  #   │   └── tramp-agent
  #   └── aarch64-unknown-linux-musl/
  #       └── tramp-agent
  #
  # Usage:
  #   nix build .#tramp-agent-cache
  #   cp -r result/* ~/.cache/nu-plugin-tramp/<version>/
  cacheLayout = {pkgs}: let
    linux = (import ./cross.nix {inherit nixpkgs;}).allLinuxFrom pkgs.system;
  in
    pkgs.runCommand "tramp-agent-cache" {} ''
      mkdir -p $out/x86_64-unknown-linux-musl
      mkdir -p $out/aarch64-unknown-linux-musl
      cp ${linux.x86_64-linux}/bin/tramp-agent $out/x86_64-unknown-linux-musl/
      cp ${linux.aarch64-linux}/bin/tramp-agent $out/aarch64-unknown-linux-musl/
    '';
}
