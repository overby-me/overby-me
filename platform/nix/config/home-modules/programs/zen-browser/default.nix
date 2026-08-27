{
  lib,
  pkgs,
  ...
}: let
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

  # Keyed by display name because the ids are opaque; the policy wants them the
  # other way round, which mapAttrs' below does.
  extensions = {
    "Absolute Enable Right Click & Copy" = "{9350bc42-47fb-4598-ae0f-825e3dd9ceba}";
    "Activate Reader View" = "@activatereaderview";
    "Altair GraphQL Client" = "{c336a627-bbea-4dbb-aa77-83899b52149a}";
    "Apollo Client Devtools" = "{a5260852-8d08-4979-8116-38f1129dfd22}";
    "Bitwarden" = "{446900e4-71c2-419f-a6a7-df9c091e268b}";
    "CORS Unblock" = "{8d9dd0f0-6dc5-4595-8c81-fab876d0fef0}";
    "Consent-O-Matic" = "gdpr@cavi.au.dk";
    "Cookie AutoDelete" = "CookieAutoDelete@kennydo.com";
    "Copyfish" = "copyfish@a9t9.com";
    "Dark Reader" = "addon@darkreader.org";
    "DeepL" = "firefox-extension@deepl.com";
    "Decentraleyes" = "jid1-BoFifL9Vbdl2zQ@jetpack";
    "Distill Web Monitor" = "{7a73dc4b-1b38-40e7-ac56-7d356dd4af34}";
    "Duplicate Tabs Closer" = "jid0-RvYT2rGWfM8q5yWxIxAHYAeo5Qg@jetpack";
    "Ecosia" = "{d04b0b40-3dab-4f0b-97a6-04ec3eddbfb0}";
    "Go European" = "goeuropean@example.com";
    "Granted Containers" = "{b5e0e8de-ebfe-4306-9528-bcc18241a490}";
    "Harper" = "harper@writewithharper.com";
    "LanguageTool" = "languagetool-webextension@languagetool.org";
    "LeechBlock NG" = "leechblockng@proginosko.com";
    "LibRedirect" = "7esoorv3@alefvanoon.anonaddy.me";
    "Progressive Web Apps for Firefox" = "firefoxpwa@filips.si";
    "Qwant" = "qwant-search-firefox@qwant.com";
    "React Developer Tools" = "@react-devtools";
    "Read on reMarkable" = "remarkable@schutter.xyz";
    "SingleFile" = "{531906d3-e22f-4a6c-a102-8057b88a1a63}";
    "Social Fixer for Facebook" = "betterfacebook@mattkruse.com";
    "SocialFocus" = "{26b4f076-089c-4c69-8497-44b7e5c9faef}";
    "Stylus" = "{7a7a4a92-a2a0-41d1-9fd7-1e92480d612d}";
    "Surfingkeys" = "{a8332c60-5b6d-41ee-bfc8-e9bb331d34ad}";
    "SwipeToNavigate" = "{bc5ae657-5db8-4f8a-b558-e7343e127fee}";
    "Tab Reloader" = "jid0-bnmfwWw2w2w4e4edvcdDbnMhdVg@jetpack";
    "Tab Wrangler" = "{81b74d53-9416-4fb3-afa2-ab46684b253b}";
    "User-Agent Switcher" = "user-agent-switcher@ninetailed.ninja";
    "Video Download Helper" = "{b9db16a4-6edc-47ec-a1f4-b86292ed211d}";
    "Video Speed Controller" = "{7be2ba16-0f1e-4d93-9ebc-5164397477a9}";
    "Web Scrobbler" = "{799c0914-748b-41df-a25c-22d008f9e83f}";
    "WhatFont" = "{dcb8caa2-63fa-41aa-a508-a45c5990ebdd}";
    "uBlock Origin" = "uBlock0@raymondhill.net";
  };

  # AMO resolves an add-on's GUID in the `latest` path, so the profile's own ids
  # are the whole input: no slug table to drift, and installs follow upstream
  # rather than pinning the version that happened to be current. Braces are the
  # one thing it will not take raw.
  #
  # normal_installed, not force_installed: several of these are switched off in
  # the profile on purpose, and force_installed removes the ability to switch
  # anything off.
  extensionSettings =
    lib.mapAttrs' (_name: id: {
      name = id;
      value = {
        install_url = "https://addons.mozilla.org/firefox/downloads/latest/${lib.replaceStrings ["{" "}"] ["%7B" "%7D"] id}/latest.xpi";
        installation_mode = "normal_installed";
      };
    })
    extensions;
in {
  programs.zen-browser = {
    enable = true;
    #nativeMessagingHosts = [pkgs.firefoxpwa];
    policies.ExtensionSettings = extensionSettings;
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

          # Zen's own chrome: compact from startup, no expanded sidebar.
          "zen.view.compact.enable-at-startup" = true;
          "zen.view.sidebar-expanded" = false;
          "zen.view.use-single-toolbar" = false;
          "sidebar.visibility" = "hide-sidebar";

          # The address bar completes history and bookmarks only. Engine and
          # open-tab rows crowd out the keyword engines above.
          "browser.urlbar.suggest.engines" = false;
          "browser.urlbar.suggest.openpage" = false;

          "browser.translations.neverTranslateLanguages" = "da";
          "browser.search.region" = "DK";
          "media.autoplay.default" = 0;
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
