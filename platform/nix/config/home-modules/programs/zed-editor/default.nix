{
  config,
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
  };
  home = {
    # Jupyter Notebook
    sessionVariables = {
      LOCAL_NOTEBOOK_DEV = 1;
    };
    activation = let
      configDir = "${config.xdg.configHome}/zed";
      settingsPath = "${configDir}/settings.json";
      keymapPath = "${configDir}/keymap.json";
      tasksPath = "${configDir}/tasks.json";

      userKeymaps = lib.readFile ./keymap.json;
      # @opencode@ stands in for the store path of the opencode binary that
      # serves ACP to the agent panel: the file stays plain JSON for editors,
      # and a running zed is insulated from PATH differences.
      userSettings = lib.replaceStrings ["@opencode@"] ["${pkgs.pkgsUnstable.opencode}/bin/opencode"] (lib.readFile ./settings.json);
      userTasks = lib.readFile ./tasks.json;
    in {
      removeExistingZedSettings = lib.hm.dag.entryBefore ["checkLinkTargets"] ''
        rm -rf "${settingsPath}" "${keymapPath}"
      '';

      overwriteZedSymlink = lib.hm.dag.entryAfter ["linkGeneration"] ''
        mkdir -p "${configDir}"
        cat ${pkgs.writeText "zed-settings" userSettings} > "${settingsPath}"
        cat ${pkgs.writeText "zed-keymaps" userKeymaps} > "${keymapPath}"
        cat ${pkgs.writeText "zed-tasks" userTasks} > "${tasksPath}"
      '';

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
