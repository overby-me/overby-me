# Flakelight module: pure-eval Rust builds from Cargo.lock. See PLAN.md.
#
# Having a default.nix also makes nix/flake/modules/lib.nix import this
# directory as a single unit instead of recursing into tests/ and index/.
{
  # Pure resolution primitives (semver, cfg, lock, index, manifest, resolve);
  # system-independent, exposed per-system for convenience.
  perSystemLib.cargoLib = _pkgs: import ./lib;

  # Repo policy: release builds link with wild by default (see the
  # benchmark section in PLAN.md). Callers can override with `linker =
  # null` (or another linker package).
  perSystemLib.buildCargoProject = pkgs: let
    f = import ./build/buildCargoProject.nix {
      inherit (pkgs) lib stdenv rustc nushell fetchurl writeText symlinkJoin runCommand;
    };
  in
    args: f ({linker = pkgs.wild;} // args);
}
