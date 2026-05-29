{lib, ...}: {
  programs.zen-browser = {
    enable = true;
    #nativeMessagingHosts = [pkgs.firefoxpwa];
    profiles = rec {
      default = {
        isDefault = true;
        settings = {
          "browser.ml.enable" = true;
          "browser.ml.chat.enabled" = true;
          "browser.ml.chat.shortcuts" = true;
          "browser.ml.chat.shortcuts.custom" = true;
          "browser.ml.chat.sidebar" = true;
          "layout.spellcheckDefault" = 0;
        };
        # Catppucccin
        userChrome = lib.readFile ./userChrome.css;
        userContent = lib.readFile ./userContent.css;
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
