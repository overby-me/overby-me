{pkgs, ...}: {
  # macOS $TMPDIR is long; use a short socket dir so Zellij's IPC path stays under the 103-byte UNIX socket limit.
  home.sessionVariables = pkgs.lib.mkIf pkgs.stdenv.isDarwin {
    ZELLIJ_SOCKET_DIR = "/tmp/zellij";
  };

  programs.zellij = {
    enable = true;
    settings = {
      default_shell = "nu";
      # No copy_command on purpose: it would run where the zellij server
      # runs, which over ssh is the wrong machine. The OSC 52 fallback
      # follows the connection to the terminal the user is looking at.
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
