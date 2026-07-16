let
  # The container binary derivation, shared by the `wiki-backend` package and the
  # OCI image below. An axum HTTP server (src/main.rs) wrapping `handle`.
  mkBackend = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "wiki-backend";
      version = "0.1.0";
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };
      cargoLock.lockFile = ./Cargo.lock;
      # Run the unit tests as part of the package build: there is no separate
      # CI executor for them (the tangled microVM cannot compile this dep
      # tree), so the Nix build is where they actually run.
      doCheck = true;
    };
in {
  devenvConfigurations.wiki-backend = {
    pkgs,
    inputs,
    ...
  }: {
    imports = with inputs.self.devenvModules; [
      devenv-root
    ];

    languages.rust.enable = true;

    packages = with pkgs; [
      openssl
      # scw: create/manage the Serverless Container + Container Registry.
      scaleway-cli
      # skopeo: push the Nix-built OCI image to a registry with no Docker daemon.
      skopeo
    ];
  };

  # The Serverless Container binary.
  packages.wiki-backend = {
    lib,
    rustPlatform,
    ...
  }:
    mkBackend {inherit lib rustPlatform;};

  # A reproducible OCI image built entirely with Nix (dockerTools), for Scaleway
  # Serverless Containers. `nix build .#wiki-backend-image` produces a docker-archive
  # tarball; push it with `skopeo copy docker-archive:result docker://<registry>`.
  # Build for linux/amd64 — Scaleway Serverless Containers does not support arm64.
  packages.wiki-backend-image = {
    lib,
    rustPlatform,
    dockerTools,
    cacert,
    ...
  }:
    dockerTools.buildLayeredImage {
      name = "wiki-backend";
      tag = "latest";
      contents = [cacert];
      config = {
        Cmd = ["${mkBackend {inherit lib rustPlatform;}}/bin/wiki-backend"];
        ExposedPorts = {"8080/tcp" = {};};
        # The container binds 0.0.0.0:$PORT (see src/main.rs).
        Env = [
          "PORT=8080"
          "SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt"
        ];
      };
    };
}
