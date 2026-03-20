{
  devShells.tangled-spindle-nix-engine = pkgs: {
    packages = with pkgs; [
      just
    ];
  };

  packages.tangled-spindle-nix-engine = {
    lib,
    rustPlatform,
    git,
  }:
    rustPlatform.buildRustPackage {
      pname = "tangled-spindle-nix-engine";
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

      nativeBuildInputs = [
        git
      ];

      doCheck = true;

      meta = {
        description = "Rust reimplementation of the Tangled Spindle CI runner with native Nix engine";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/tangled/spindle-nix-engine";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [noverby];
        mainProgram = "tangled-spindle";
      };
    };

  nixosModules.tangled-spindle-nix-engine = ./nixos-module.nix;

  checks.tangled-spindle-nix-engine-integration = pkgs:
    import ./nixos-test.nix {
      inherit pkgs;
      tangled-spindle-nix-engine = pkgs.tangled-spindle-nix-engine or (throw "tangled-spindle-nix-engine package not found");
    };
}
