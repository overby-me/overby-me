{
  packages = {
    rust-flatpak = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-flatpak";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        meta = {
          description = "A Flatpak-compatible application sandboxing and distribution tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/flatpak";
          license = lib.licenses.mit;
          mainProgram = "flatpak";
          platforms = lib.platforms.linux;
        };
      };

    rust-flatpak-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-flatpak-dev";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        buildType = "debug";

        meta = {
          description = "A Flatpak-compatible application sandboxing and distribution tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/flatpak";
          license = lib.licenses.mit;
          mainProgram = "flatpak";
          platforms = lib.platforms.linux;
        };
      };
  };

  checks = let
    testNames = [
      "build-bundle-basic"
      "build-bundle-missing-args"
      "build-bundle-roundtrip"
      "build-commit-from-missing-args"
      "build-export-branch"
      "build-export-creates-repo"
      "build-export-multiple-apps"
      "build-export-no-dir"
      "build-export-subject"
      "build-finish-command"
      "build-finish-export-dbus"
      "build-finish-export-desktop"
      "build-finish-export-icons"
      "build-finish-export-metainfo"
      "build-finish-idempotent"
      "build-finish-no-dir"
      "build-finish-permissions"
      "build-import-bundle-missing-args"
      "build-init"
      "build-init-custom-branch"
      "build-init-dirs"
      "build-init-extension"
      "build-init-metadata-format"
      "build-init-missing-args"
      "build-init-no-extra-files"
      "build-missing-args"
      "build-sign-missing-args"
      "build-update-repo"
      "build-update-repo-creates-summary"
      "build-update-repo-missing-args"
      "config-list"
      "config-user"
      "config-user-path"
      "create-usb-missing-args"
      "enter-missing-args"
      "global-user-flag"
      "global-verbose-flag"
      "help"
      "help-commands"
      "help-exit-zero"
      "help-usage-format"
      "history-empty"
      "info-arch"
      "info-branch"
      "info-command"
      "info-installation"
      "info-installed-app"
      "info-location"
      "info-missing-args"
      "info-ref-format"
      "info-runtime"
      "info-show-metadata"
      "info-show-permissions"
      "install-creates-data-dirs"
      "install-dir-with-export"
      "install-from-dir"
      "install-missing-args"
      "install-multiple-apps"
      "install-preserves-metadata"
      "install-then-list"
      "kill-missing-args"
      "list-empty"
      "list-filter-app"
      "list-header-format"
      "list-installed-app"
      "list-runtime-filter"
      "mask-missing-args"
      "mask-pattern"
      "metadata-comments-ignored"
      "metadata-empty-groups"
      "metadata-multiline-groups"
      "metadata-parse"
      "metadata-roundtrip"
      "missing-command"
      "override-context-accumulate"
      "override-device"
      "override-device-multiple"
      "override-env"
      "override-env-overwrite"
      "override-file-format"
      "override-filesystem"
      "override-global-overrides-dir"
      "override-missing-app"
      "override-multiple-env"
      "override-multiple-filesystems"
      "override-multiple-sockets"
      "override-reset"
      "override-separate-apps"
      "override-share"
      "override-share-multiple"
      "override-show"
      "override-socket"
      "pin-missing-args"
      "pin-pattern"
      "remote-add"
      "remote-add-duplicate"
      "remote-add-flatpakrepo-url"
      "remote-add-from-file"
      "remote-add-missing-url"
      "remote-add-multiple"
      "remote-add-order-preserved"
      "remote-add-then-list-url"
      "remote-add-title"
      "remote-delete"
      "remote-delete-missing"
      "remote-info-missing-args"
      "remote-list"
      "remote-ls-missing-args"
      "remote-modify-implicit"
      "repair-no-crash"
      "repo-info"
      "run-missing-app"
      "run-missing-args"
      "search-missing-args"
      "search-no-remote"
      "uninstall-app"
      "uninstall-delete-data"
      "uninstall-missing-args"
      "uninstall-nonexistent"
      "unknown-command"
      "update-no-remotes"
      "version"
      "version-format"
      "version-nonzero-exit"
    ];

    vmTestNames = [
      "run-hello"
      "run-command-override"
      "run-nonexistent"
      "run-flatpak-info"
      "run-xdg-dirs"
      "run-env-override"
      "run-missing-args"
      "config-set-get"
      "override-env-sandbox"
      "one-dir-commands"
    ];
  in
    builtins.listToAttrs (map (name: {
        name = "rust-flatpak-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames)
    // builtins.listToAttrs (map (name: {
        name = "rust-flatpak-vm-${name}";
        value = pkgs: import ./vmtest.nix {inherit pkgs name;};
      })
      vmTestNames);
}
