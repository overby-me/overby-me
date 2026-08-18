_: {
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
