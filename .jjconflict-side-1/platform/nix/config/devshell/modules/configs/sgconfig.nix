# sgconfig.yml, generated because one line of it is a Nix store path.
#
# ast-grep finds a custom grammar by dlopening an absolute path, and the Mojo
# grammar is built from source here rather than shipped by nixpkgs. Writing that
# path by hand would pin one machine's store, so the file is generated the same
# way .commitlintrc.yml is and copied into the root on shell entry.
{
  pkgs,
  lib,
  ...
}:
pkgs.writeText "sgconfig.yml" ''
  # GENERATED from platform/nix/config/devshell/modules/configs/sgconfig.nix.
  # Edit that file, then `touch .envrc && direnv export json`.
  #
  # Only Mojo is registered here, and only because ast-grep has no Mojo built
  # in. Nix, Rust and the rest must NOT be added: registering a name that
  # already exists built-in makes every rule for it silently match nothing,
  # because rules resolve the name through the built-in table and files resolve
  # it through the custom extension map, and the two compare unequal. That
  # failure is silent - the scan reports zero findings and exits 0.
  ruleDirs:
    - dev/ast-grep/rules

  customLanguages:
    mojo:
      libraryPath: ${lib.getOutput "out" pkgs.tree-sitter-mojo}/lib/mojo.so
      extensions: [mojo]
      # ast-grep rewrites a metavariable to an identifier before handing the
      # pattern to tree-sitter, so the placeholder has to be spellable as one.
      expandoChar: _
''
