# Workspace module: exposes the pure-Nix Starlark (Skylark) interpreter as
# perSystemLib.skylark (system-independent, but exposed per-system for
# convenience, like platform/nix/lib/lib/cargo's cargoLib). The real API lives in
# ./api.nix; Buck2 (platform/nix/lib/lib/buck2) imports it directly by path.
#
# Having a default.nix also makes the platform/nix/lib/lib autoloader import this directory
# as a single unit instead of recursing into tests/.
{
  perSystemLib.skylark = _pkgs: import ./api.nix;
}
