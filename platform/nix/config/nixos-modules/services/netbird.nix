{
  config,
  lib,
  ...
}: {
  # Pre-login SSH to armitas.netbird.cloud holds only while login expiration
  # stays off for this peer in the NetBird dashboard: with it on the daemon
  # drops to NeedsLogin and blocks on a browser SSO flow that needs the very
  # graphical session you were trying to reach. The alternative is enrolling
  # with a setup key (`clients.default.login.enable` + `login.setupKeyFile`).
  services.netbird.enable = true;

  # graphical-session.target is the right anchor on this host: COSMIC's
  # cosmic-session.target is `BindsTo=` it, so the applet follows login and
  # logout. The wrapper behind services.netbird.ui.enable is already pinned to
  # the default client's socket, hence no --daemon-addr here.
  systemd.user.services.netbird-ui = lib.mkIf config.services.netbird.ui.enable {
    description = "NetBird tray applet";
    wantedBy = ["graphical-session.target"];
    partOf = ["graphical-session.target"];
    after = ["graphical-session.target"];
    # The SSO flow shells out to xdg-open, which resolves the browser off PATH.
    # The NixOS default `Environment="PATH=coreutils:…"` would replace the PATH
    # cosmic-session imported into the user manager, hiding both from the
    # applet. Same reasoning as nixpkgs' niri module.
    enableDefaultPath = false;
    serviceConfig = {
      ExecStart = lib.getExe' config.services.netbird.clients.default.wrapper "netbird-ui";
      Restart = "on-failure";
      RestartSec = 3;
    };
  };
}
