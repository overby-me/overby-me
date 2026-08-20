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
      userSettings = lib.readFile ./settings.json;
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

      # Dev Extensions - copied (not symlinked) so Zed can write build artifacts
      installZedDevExtensions = lib.hm.dag.entryAfter ["linkGeneration"] ''
        dev_ext_dir="$HOME/.local/share/zed/dev_extensions"
        mkdir -p "$dev_ext_dir"

        rm -rf "$dev_ext_dir/mojo"
        cp -rL ${inputs.self.zedExtensions.mojo-zed} "$dev_ext_dir/mojo"
        chmod -R u+w "$dev_ext_dir/mojo"

        rm -rf "$dev_ext_dir/nickel"
        cp -rL ${inputs.self.zedExtensions.nickel-zed} "$dev_ext_dir/nickel"
        chmod -R u+w "$dev_ext_dir/nickel"
      '';
    };
  };
}
