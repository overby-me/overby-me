{
  config,
  lib,
  ...
}: {
  services.netbird.enable = true;

  # Autostart the NetBird tray applet with the desktop session. COSMIC's
  # cosmic-session.target is `BindsTo=graphical-session.target`, so this comes
  # up right after login and goes away again on logout.
  #
  # `services.netbird.ui.enable` defaults to true whenever the host has a
  # graphical session, and is what builds the `netbird-ui` wrapper that is
  # already pinned to the default client's daemon socket
  # (`--daemon-addr=unix:///var/run/netbird/sock`).
  systemd.user.services.netbird-ui = lib.mkIf config.services.netbird.ui.enable {
    description = "NetBird tray applet";
    wantedBy = ["graphical-session.target"];
    partOf = ["graphical-session.target"];
    after = ["graphical-session.target"];
    # The SSO login flow shells out to xdg-open, which in turn resolves the
    # browser off PATH. The NixOS default `Environment="PATH=coreutils:…"`
    # would replace the PATH that cosmic-session imported into the user
    # manager, hiding both from the applet, so opt out of it and inherit the
    # session's own PATH instead (same reasoning as nixpkgs' niri module).
    enableDefaultPath = false;
    serviceConfig = {
      ExecStart = lib.getExe' config.services.netbird.clients.default.wrapper "netbird-ui";
      Restart = "on-failure";
      RestartSec = 3;
    };
  };
}
