# The AppView deploy target (rewrite kickoff item 11). Unlike the interim backend
# (backend/default.nix, a scale-to-zero Scaleway Serverless Container), the
# AppView is a SINGLE STATEFUL always-on process: it holds the Turso core+view,
# a live firehose connection, the in-process broadcast channel, and the WebSocket
# server. It therefore needs a persistent-process host (a VM/bare-metal systemd
# unit behind Caddy), not serverless. This file provides the native binary
# package; `./nixos-module.nix` is the systemd/NixOS unit that runs it.
{
  # The native appview binary, built from the `crates/` workspace. Depends on the
  # atrium-oauth tree (reqwest -> TLS) and turso, so it needs pkg-config + openssl
  # at build time.
  packages.wiki-appview = {
    lib,
    rustPlatform,
    pkg-config,
    openssl,
    ...
  }:
    rustPlatform.buildRustPackage {
      pname = "wiki-appview";
      version = "0.1.0";

      # The whole rewrite workspace is the source: the appview crate depends on
      # sibling members (domain-types, schema, ballot-spec), so the build needs
      # all of them plus the shared lockfile.
      src = lib.fileset.toSource {
        root = ./..;
        fileset = lib.fileset.unions [
          ./../Cargo.toml
          ./../Cargo.lock
          ./../appview
          ./../ballot-spec
          ./../ballot-store
          ./../dagcbor-spike
          ./../domain-types
          ./../durability-harness
          ./../migration-extractor
          ./../migration-loader
          ./../oauth-spike
          ./../schema
        ];
      };

      cargoLock.lockFile = ./../Cargo.lock;

      # Build (and install) ONLY the appview binary out of the workspace.
      cargoBuildFlags = ["--package" "appview"];
      buildAndTestSubdir = null;

      nativeBuildInputs = [pkg-config];
      buildInputs = [openssl];

      # The workspace unit tests run locally and in the migration crates' own
      # checks; the deploy build only produces the serving binary (some workspace
      # tests spawn processes / SIGKILL, which do not belong in a package build).
      doCheck = false;

      meta = {
        description = "RadikalWiki atproto AppView (stateful axum + Turso + firehose)";
        mainProgram = "appview";
      };
    };

  # The stateful systemd service module (a host imports this + enables it).
  nixosModules.wiki-appview = ./nixos-module.nix;

  # End-to-end VM test: the acceptance harness for the always-on process, too big
  # for a derivation check (it boots a real VM behind Caddy and soaks a restart).
  # `nix build .#checks.<system>.wiki-appview-e2e`.
  checks.wiki-appview-e2e = pkgs:
    import ./nixos-test.nix {
      inherit pkgs;
      wiki-appview = pkgs.wiki-appview or (throw "wiki-appview package not found");
    };
}
