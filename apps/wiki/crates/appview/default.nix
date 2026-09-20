# The AppView deploy target (rewrite kickoff item 11). Unlike the interim backend
# (backend/default.nix, a scale-to-zero Scaleway Serverless Container), the
# AppView is a SINGLE STATEFUL always-on process: it holds the Turso core+view,
# a live firehose connection, the in-process broadcast channel, and the WebSocket
# server. It therefore needs a persistent-process host (a VM/bare-metal systemd
# unit behind Ferron), not serverless. This file provides the native binary
# package; `./nixos-module.nix` is the systemd/NixOS unit that runs it.
let
  # The whole rewrite workspace is the source: a member depends on its siblings
  # (domain-types, schema, ballot-spec), and cargo reads every member's manifest,
  # so a build needs all of them plus the shared lockfile.
  workspace = lib:
    lib.fileset.toSource {
      root = ./..;
      fileset = lib.fileset.unions [
        ./../Cargo.toml
        ./../Cargo.lock
        ./../appview
        ./../appview-client
        ./../appview-dev
        ./../ballot-spec
        ./../ballot-store
        ./../board-mirror
        ./../dagcbor-spike
        ./../domain-types
        ./../durability-harness
        ./../fake-pds
        ./../fake-plc
        ./../lexgen
        ./../migration-extractor
        ./../migration-loader
        ./../oauth-spike
        ./../schema
      ];
    };
  cargoLock = {
    lockFile = ./../Cargo.lock;
    # The one git dependency (the metafile renderer's fork, pinned to a rev
    # in appview/Cargo.toml). The interim backend pins the same rev.
    outputHashes = {
      "emfsdk-0.2.0" = "sha256-UvdZXTczvGL2vRl4CWfTXKiFjLBaM5Uvup9+62YJ8O8=";
    };
  };
in {
  # The native appview binary, built from the `crates/` workspace. TLS is rustls
  # throughout (src/http.rs), so the build needs no OpenSSL and no pkg-config.
  packages.wiki-appview = {
    lib,
    rustPlatform,
    ...
  }:
    rustPlatform.buildRustPackage {
      pname = "wiki-appview";
      version = "0.1.0";
      src = workspace lib;
      inherit cargoLock;

      # Build (and install) ONLY the appview binary out of the workspace.
      cargoBuildFlags = ["--package" "appview"];
      buildAndTestSubdir = null;

      # The workspace unit tests run locally and in the migration crates' own
      # checks; the deploy build only produces the serving binary (some workspace
      # tests spawn processes / SIGKILL, which do not belong in a package build).
      doCheck = false;

      meta = {
        description = "wiki atproto AppView (stateful axum + Turso + firehose)";
        mainProgram = "appview";
      };
    };

  # The independent copy of a published ballot board, and the check of it. A
  # package of its own because whoever runs it is NOT the host of the AppView:
  # it is for someone outside the organization (`docs/ballot-board-custody.md`).
  packages.wiki-board-mirror = {
    lib,
    rustPlatform,
    ...
  }:
    rustPlatform.buildRustPackage {
      pname = "wiki-board-mirror";
      version = "0.1.0";
      src = workspace lib;
      inherit cargoLock;
      cargoBuildFlags = ["--package" "board-mirror"];
      buildAndTestSubdir = null;
      doCheck = false;

      meta = {
        description = "Follows and recounts a published wiki ballot board";
        mainProgram = "board-mirror";
      };
    };

  # The stateful systemd service module (a host imports this + enables it).
  nixosModules.wiki-appview = ./nixos-module.nix;

  # End-to-end VM test: the acceptance harness for the always-on process, too big
  # for a derivation check (it boots a real VM behind Ferron and soaks a restart).
  # `nix build .#checks.<system>.wiki-appview-e2e`.
  checks.wiki-appview-e2e = pkgs:
    import ./nixos-test.nix {
      inherit pkgs;
      wiki-appview = pkgs.wiki-appview or (throw "wiki-appview package not found");
    };
}
