# Standalone build for the published repo. The same directory is a discovered
# project of the monorepo; this flake exists so a clone can run the check that
# the monorepo runs, against the same framework and the same grammar.
#
# Workspace form - `inputs`, not `name` - because the single-project form
# builds a crate, and this repo is rules. The framework walks the tree and
# finds the project in ./check; the root itself is never one.
#
# nix-packages is a source, not a flake: one package is called out of it, the
# patched tree-sitter-mojo grammar the Mojo rules load. Taken as a flake it
# would bring its whole package set along as checks, and their toolchains
# with them.
{
  description = "Structural ast-grep rules for Rust, Nix and Mojo, with fixture tests and a pinned Mojo grammar";

  inputs = {
    workspace.url = "git+https://tangled.org/overby.me/nix-workspace";

    nix-packages = {
      url = "git+https://tangled.org/overby.me/nix-packages";
      flake = false;
    };
  };

  outputs = inputs:
    inputs.workspace ./. {
      inherit inputs;
      withOverlays = [
        (final: _prev: {
          tree-sitter-mojo =
            final.callPackage "${inputs.nix-packages}/packages/tree-sitter-mojo" {};
        })
      ];
    };
}
