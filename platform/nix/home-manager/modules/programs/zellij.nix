{pkgs, ...}: {
  # macOS $TMPDIR is long; use a short socket dir so Zellij's IPC path stays under the 103-byte UNIX socket limit.
  home.sessionVariables = pkgs.lib.mkIf pkgs.stdenv.isDarwin {
    ZELLIJ_SOCKET_DIR = "/tmp/zellij";
  };

  programs.zellij = {
    enable = true;
    settings = {
      default_shell = "nu";
      copy_command =
        if pkgs.stdenv.isDarwin
        then "pbcopy"
        else "${pkgs.pkgsUnstable.wclip}/bin/wclip -selection clipboard";
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
