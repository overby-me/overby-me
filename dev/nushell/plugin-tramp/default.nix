{
  devShells.nushell-plugin-tramp = pkgs: {
    packages = with pkgs; [
      just
      openssh
    ];
  };

  packages = {
    nushell-plugin-tramp = {
      lib,
      rustPlatform,
      openssh,
    }:
      rustPlatform.buildRustPackage {
        pname = "nushell-plugin-tramp";
        version = "unstable";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./crates
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        nativeCheckInputs = [
          openssh
        ];

        doCheck = false;

        meta = {
          description = "A TRAMP-inspired remote filesystem plugin for Nushell";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/nushell/plugin-tramp";
          license = lib.licenses.mit;
          maintainers = with lib.maintainers; [overby-me];
          mainProgram = "nu_plugin_tramp";
        };
      };

    nushell-plugin-tramp-agent = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "tramp-agent";
        version = "unstable";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./crates
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        cargoBuildFlags = ["-p" "tramp-agent"];
        cargoTestFlags = ["-p" "tramp-agent"];

        # Use the size-optimised release profile for the agent binary
        CARGO_PROFILE = "release-agent";

        meta = {
          description = "Lightweight RPC agent for nushell-plugin-tramp remote filesystem operations";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/nushell/plugin-tramp";
          license = lib.licenses.mit;
          maintainers = with lib.maintainers; [overby-me];
          mainProgram = "tramp-agent";
        };
      };

    nushell-plugin-tramp-agent-cache = pkgs: let
      cross = import ./cross.nix {
        nixpkgs = pkgs.path;
        inherit (pkgs) lib;
      };
      linux = cross.allLinuxFrom "x86_64-linux";
    in
      pkgs.runCommand "tramp-agent-cache" {} ''
        mkdir -p $out/x86_64-unknown-linux-musl
        mkdir -p $out/aarch64-unknown-linux-musl
        cp ${linux.x86_64-linux}/bin/tramp-agent $out/x86_64-unknown-linux-musl/
        cp ${linux.aarch64-linux}/bin/tramp-agent $out/aarch64-unknown-linux-musl/
      '';
  };

  homeModules.nushell-plugin-tramp = ./hm-module.nix;
}
