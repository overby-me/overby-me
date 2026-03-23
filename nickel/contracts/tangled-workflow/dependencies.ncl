# Dependencies contract for tangled workflows
#
# Dependencies are keyed by Nix registry and map to lists of package names.
# The engine resolves these packages and makes them available during execution.
#
# Examples:
#   nixpkgs: [go, gcc]
#   nixpkgs/nixpkgs-unstable: [bun]
#   git+https://tangled.org/@example.com/my_pkg: [my_pkg]
{
  Dependencies
    | doc m%"
      Map of Nix registry to package lists.
      Keys are registry references (e.g. "nixpkgs", "nixpkgs/nixpkgs-unstable").
      Values are arrays of package attribute names to install.
    "%
    = { _ : Array String },
}
