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
  devShells.wiki-backend = pkgs: {
    packages = with pkgs; [
      cargo
      rustc
      rust-analyzer
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
    runCommand,
    liberation_ttf,
    carlito,
    ...
  }: let
    # Fonts for the metafile renderer (src/metafile.rs). `fontdb` reads
    # `/usr/share/fonts` and nothing else on Linux, so a package under
    # `contents` would land at `/share/fonts` and be invisible: a pasted table
    # would render as ruled lines with no words in them.
    #
    # Metric-compatible substitutes, not just any fonts: Office documents ask
    # for Arial and Calibri, and a metafile positions each text run itself, so a
    # face of different widths puts the words in the wrong places rather than
    # merely looking different. Liberation Sans matches Arial, Carlito matches
    # Calibri.
    fonts = runCommand "wiki-backend-fonts" {} ''
      mkdir -p $out/usr/share/fonts
      cp ${liberation_ttf}/share/fonts/truetype/*.ttf $out/usr/share/fonts/
      cp ${carlito}/share/fonts/truetype/*.ttf $out/usr/share/fonts/
    '';
  in
    dockerTools.buildLayeredImage {
      name = "wiki-backend";
      tag = "latest";
      contents = [cacert fonts];
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
