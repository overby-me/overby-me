{pkgs, ...}: let
  nixIcon = "${pkgs.nixos-icons}/share/icons/hicolor/scalable/apps/nix-snowflake.svg";

  engine = name: alias: template: extra:
    {
      inherit name;
      urls = [{inherit template;}];
      definedAliases = [alias];
    }
    // extra;

  # `force` is what makes any of this stick: Zen rewrites the search config on
  # every launch otherwise. It also replaces whatever is there, so the engines
  # worth keeping are declared here too; an empty attrset means the built-in of
  # that id, which is how Qwant survives being the default.
  search = {
    force = true;
    default = "qwant";
    engines = {
      qwant = {};
      ddg = {};
      wikipedia = {};

      nixpkgs = engine "Nixpkgs" "!pkgs" "https://search.nixos.org/packages?query={searchTerms}" {icon = nixIcon;};
      nixos-options = engine "NixOS Options" "!nix" "https://search.nixos.org/options?query={searchTerms}" {icon = nixIcon;};
      home-manager-options = engine "Home Manager Options" "!hm" "https://home-manager-options.extranix.com/?query={searchTerms}" {icon = nixIcon;};
    };
  };
in {
  programs.zen-browser = {
    enable = true;
    #nativeMessagingHosts = [pkgs.firefoxpwa];
    profiles = rec {
      default = {
        isDefault = true;
        inherit search;
        settings = {
          "browser.ml.enable" = true;
          "browser.ml.chat.enabled" = true;
          "browser.ml.chat.shortcuts" = true;
          "browser.ml.chat.shortcuts.custom" = true;
          "browser.ml.chat.sidebar" = true;
          "layout.spellcheckDefault" = 0;
          # Hardware-accelerated video decoding via VA-API (Intel iHD iGPU).
          # Without these, Zen falls back to multi-threaded ffmpeg software
          # H.264 decode, which can saturate ~17 cores on video-heavy pages.
          "media.ffmpeg.vaapi.enabled" = true;
          "media.hardware-video-decoding.force-enabled" = true;
        };
      };
      dev =
        default
        // {
          id = 1;
          isDefault = false;
        };
    };
  };
}
