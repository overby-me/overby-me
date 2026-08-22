{
  package = {
    lib,
    rustPlatform,
    pkg-config,
    openssl,
    dbus,
  }:
    rustPlatform.buildRustPackage {
      pname = "tangled-cli";
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
        pkg-config
      ];

      buildInputs = [
        openssl
        dbus
      ];

      cargoBuildFlags = ["-p" "tangled-cli"];

      # Integration tests require network access and a running server
      doCheck = false;

      meta = {
        description = "Rust CLI for Tangled, a decentralized git collaboration platform built on the AT Protocol";
        homepage = "https://tangled.org/vitorpy.com/tangled-cli";
        license = with lib.licenses; [mit asl20];
        maintainers = with lib.maintainers; [overby-me];
        mainProgram = "tangled-cli";
      };
    };
}
