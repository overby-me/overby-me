# ShellConfig records into the mkShell derivations behind devShells. A loaded
# plugin may contribute extra packages of its own: the Rust plugin adds
# cargo/rustc/clippy when `rust-toolchain` is set. A ShellConfig looks like
#   {
#     packages = ["cargo" "rustc" "rust-analyzer"];
#     env = { RUST_LOG = "debug"; };
#     shell-hook = "echo 'welcome'";
#     tools = { rust-analyzer = ""; };
#     inputs-from = ["my-tool"];
#     systems = null;  # optional override
#   }
{lib}: let
  # Build a single devShell derivation from a ShellConfig.
  buildShell = pkgs: name: shellConfig: workspacePackages: pluginExtras: let
    # A name is a dot-path attribute lookup into pkgs: "cargo" → pkgs.cargo.
    resolvePackage = attrName: let
      parts = lib.splitString "." attrName;
    in
      lib.attrByPath parts
      (throw "nix-workspace: shell '${name}' references unknown package '${attrName}' — not found in nixpkgs")
      pkgs;

    shellPackages =
      map resolvePackage (shellConfig.packages or []);

    # { name = version; } pairs. The version is ignored - nixpkgs' is always
    # what is used - so the field is only there for pinning later.
    toolPackages = lib.mapAttrsToList (
      toolName: _version:
        resolvePackage toolName
    ) (shellConfig.tools or {});

    inputsFromPackages = map (
      pkgName:
        workspacePackages.${pkgName}
          or (throw "nix-workspace: shell '${name}' has inputs-from '${pkgName}' but no such package exists in the workspace")
    ) (shellConfig.inputs-from or []);

    envVars = shellConfig.env or {};

    shellHook = shellConfig.shell-hook or "";
  in
    pkgs.mkShell (
      {
        inherit shellHook;

        name = "nix-workspace-${name}";

        packages = shellPackages ++ toolPackages ++ pluginExtras;

        inputsFrom = inputsFromPackages;
      }
      // envVars
    );

  # Build all shells for a given system.
  #
  # Returns:
  #   { name = derivation; ... } suitable for devShells.${system}
  buildShells = pkgs: shellConfigs: workspacePackages: pluginExtras:
    lib.mapAttrs (
      name: cfg:
        buildShell pkgs name cfg workspacePackages pluginExtras
    )
    shellConfigs;
in {
  inherit buildShell buildShells;
}
