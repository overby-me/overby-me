{
  lib,
  pkgs,
  ...
}: {
  xdg = {
    enable = true;
    # xdg.mimeApps writes .desktop association files, which only exist on
    # Linux desktops. On Darwin, default-app handling is managed by
    # LaunchServices instead.
    mimeApps = lib.mkIf pkgs.stdenv.isLinux {
      enable = true;
      associations.added = let
        zenMimes = [
          "x-scheme-handler/http"
          "x-scheme-handler/https"
          "x-scheme-handler/chrome"
          "text/html"
          "application/x-extension-htm"
          "application/x-extension-html"
          "application/x-extension-shtml"
          "application/xhtml+xml"
          "application/x-extension-xhtml"
          "application/x-extension-xht"
        ];
      in
        lib.listToAttrs (
          map (mime: {
            name = mime;
            value = "zen-beta.desktop";
          })
          zenMimes
        );
      defaultApplications = let
        zedMimes = [
          "text/plain"
          "text/markdown"
          "text/x-markdown"
          "text/x-python"
          "text/x-script.python"
          "text/x-c"
          "text/x-c++"
          "text/x-java"
          "text/javascript"
          "text/x-javascript"
          "text/x-typescript"
          "text/vnd.trolltech.linguist"
          "application/x-tiled-tsx"
          "text/rust"
          "text/x-rust"
          "text/x-go"
          "text/x-shellscript"
          "text/x-scala"
          "text/x-ruby"
          "text/x-perl"
          "text/x-log"
          "text/x-makefile"
          "text/x-csrc"
          "text/x-chdr"
          "text/x-c++src"
          "text/x-c++hdr"
          "text/x-yaml"
          "text/x-toml"
          "application/toml"
          "text/xml"
          "text/json"
          "application/json"
          "application/yaml"
          "application/x-yaml"
          "application/xml"
          "application/javascript"
          "application/x-shellscript"
          "application/x-perl"
          "application/x-ruby"
          "application/x-python"
        ];
      in
        lib.listToAttrs (
          map (mime: {
            name = mime;
            value = "dev.zed.Zed.desktop";
          })
          zedMimes
        );
    };
  };
}
