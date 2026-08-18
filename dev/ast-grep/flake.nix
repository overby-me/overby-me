# Standalone build for the published repo. The same directory is a discovered
# project of the monorepo; this flake exists so a clone can run the check that
# the monorepo runs, against the same framework and the same grammar.
#
# nix-packages is here for one package: the patched tree-sitter-mojo grammar
# the Mojo rules load. It follows this flake's workspace so the two resolve
# one framework and one nixpkgs between them.
{
  description = "Structural ast-grep rules for Rust, Nix and Mojo, with fixture tests and a pinned Mojo grammar";

  inputs = {
    workspace.url = "git+https://tangled.org/overby.me/nix-workspace";

    nix-packages = {
      url = "git+https://tangled.org/overby.me/nix-packages";
      inputs.workspace.follows = "workspace";
    };
  };

  outputs = inputs:
    inputs.workspace ./. {
      inherit inputs;
    };
}
