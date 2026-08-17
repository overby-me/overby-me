{pkgs, ...}: {
  systemd.user = {
    services = {
      vibe = {
        Unit = {
          Description = "Vibe desktop audio visualizer";
          After = ["graphical-session.target"];
          Wants = ["graphical-session.target"];
        };
        Service = {
          Type = "exec";
          ExecStart = "${pkgs.vibe}/bin/vibe";
          Restart = "on-failure";
          RestartSec = "5s";
        };
        Install = {
          WantedBy = ["graphical-session.target"];
        };
      };
    };
  };
}
