{pkgs, ...}: {
  programs.zellij = {
    enable = true;
    settings = {
      default_shell = "nu";
      copy_command =
        if pkgs.stdenv.isDarwin
        then "pbcopy"
        else "wl-copy";
      scrollback_editor = "zed-uf";
      session_serialization = false;
      pane_frames = false;
      show_startup_tips = false;
      env = {
        TERM = "tmux-256color";
      };
    };
  };
}
