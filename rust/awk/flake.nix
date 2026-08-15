# Standalone build for the published rust-awk repo.
#
# This is deliberately not the monorepo's default.nix. That one is a
# flakelight module the root flake imports, and it reaches for
# ../../nix/lib/cargo, so it means nothing outside this tree. This builds the
# crate with plain nixpkgs so a clone of the published repo works on its own.
{
  description = "A GNU awk-compatible text processing tool written in Rust";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs = {
    self,
    nixpkgs,
  }: let
    systems = ["x86_64-linux" "aarch64-linux" "aarch64-darwin"];
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
  in {
    packages = forAllSystems (pkgs: rec {
      rust-awk = pkgs.rustPlatform.buildRustPackage {
        pname = "rust-awk";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;

        postInstall = ''
          ln -s $out/bin/awk $out/bin/gawk
        '';

        meta = {
          description = "A GNU awk-compatible text processing tool written in Rust";
          homepage = "https://tangled.org/overby.me/rust-awk";
          license = pkgs.lib.licenses.mit;
          mainProgram = "awk";
        };
      };
      default = rust-awk;
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        inputsFrom = [self.packages.${pkgs.stdenv.hostPlatform.system}.rust-awk];
        packages = with pkgs; [cargo rustc rust-analyzer clippy rustfmt];
      };
    });

    checks = forAllSystems (pkgs: {
      inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) rust-awk;

      clippy =
        pkgs.runCommand "rust-awk-clippy" {
          nativeBuildInputs = with pkgs; [cargo clippy rustc rustPlatform.cargoSetupHook];
          cargoDeps = pkgs.rustPlatform.importCargoLock {lockFile = ./Cargo.lock;};
        } ''
          cp -r ${./.} src && chmod -R u+w src && cd src
          cargo clippy --all-targets -- -D warnings
          touch $out
        '';
    });

    formatter = forAllSystems (pkgs: pkgs.alejandra);
  };
}
