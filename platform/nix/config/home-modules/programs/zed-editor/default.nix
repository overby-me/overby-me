{
  inputs,
  lib,
  pkgs,
  ...
}: {
  programs.zed-editor = {
    enable = true;
    package = pkgs.pkgsUnstable.zed-editor;
    # Rust toolchain for compiling WASM dev extensions. rust-bin exists only
    # when the evaluating tree declares rust-overlay; without it zed still
    # works, minus the wasip2 toolchain. latest, not a pin: the version
    # belongs to whoever declares the input and locks it - a pin here would
    # put a version floor on an input this tree does not even declare.
    # Recheck at nixpkgs bumps whether wasm32-wasip2 arrived and rust-bin
    # can retire entirely.
    extraPackages = with pkgs;
      [clang]
      ++ lib.optionals (pkgs ? rust-bin) [
        (rust-bin.stable.latest.default.override {
          targets = ["wasm32-wasip2"];
        })
      ];
    extensions = [
      "biome"
      "nix"

      "typos"
      "nu"
      "just"
      "just-ls"
      "cargo-appraiser"
      "cargo-tom"
      "harper"
      "jj-lsp"
      "meson"
    ];

    # The @opencode@ and @goose@ placeholders in settings.json are overwritten
    # here rather than substituted into the text: it keeps the file plain JSON
    # for editors, insulates a running zed from PATH differences, and a store
    # path may not pass through importJSON, which rejects strings with context.
    # Both serve the agent panel over ACP on stdio.
    userSettings = lib.recursiveUpdate (lib.importJSON ./settings.json) {
      agent_servers = {
        OpenCode.command = "${pkgs.pkgsUnstable.opencode}/bin/opencode";
        Goose.command = "${pkgs.pkgsUnstable.goose-cli}/bin/goose";
      };
    };
    userKeymaps = lib.importJSON ./keymap.json;
    userTasks = lib.importJSON ./tasks.json;
  };
  home = {
    # Jupyter Notebook
    sessionVariables = {
      LOCAL_NOTEBOOK_DEV = 1;
    };
    activation = {
      # Dev Extensions - copied (not symlinked) so Zed can write build
      # artifacts. Built by monorepo projects, so a standalone evaluation of
      # this tree has none and installs none.
      installZedDevExtensions = lib.hm.dag.entryAfter ["linkGeneration"] ''
        dev_ext_dir="$HOME/.local/share/zed/dev_extensions"
        mkdir -p "$dev_ext_dir"
        ${lib.concatStrings (lib.mapAttrsToList (name: ext: ''
            rm -rf "$dev_ext_dir/${name}"
            cp -rL ${ext} "$dev_ext_dir/${name}"
            chmod -R u+w "$dev_ext_dir/${name}"
          '') (let
            exts = inputs.self.zedExtensions or {};
          in
            lib.optionalAttrs (exts ? mojo-zed) {mojo = exts.mojo-zed;}
            // lib.optionalAttrs (exts ? nickel-zed) {nickel = exts.nickel-zed;}))}
      '';
    };
  };
}
