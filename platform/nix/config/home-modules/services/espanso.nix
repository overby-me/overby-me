{
  pkgs,
  lib,
  ...
}: {
  # home-manager installs espanso into default.target, so it starts whenever
  # the user manager does, which includes an SSH login on a machine nobody has
  # logged into graphically.  Its EVDEV detector then asks the compositor for
  # modifier state, finds none, and takes the worker down with it:
  #
  #   [ERROR] thread 'detect thread' panicked at 'called `Result::unwrap()` on
  #           an `Err` value: NoCompositor': espanso-detect/src/evdev/sync/wayland.rs:42
  #
  # systemd restarts it and it loops for as long as the session lasts.  Bind
  # it to the desktop instead, the same way platform/nix/config/nixos-modules/services/netbird.nix
  # handles its tray applet.
  systemd.user.services.espanso = {
    Unit = {
      After = ["graphical-session.target"];
      PartOf = ["graphical-session.target"];
    };
    Install.WantedBy = lib.mkForce ["graphical-session.target"];
  };

  services.espanso = {
    enable = true;

    # Both Linux hosts run COSMIC, which is Wayland-only.  The default
    # `espanso` build is the X11 one: it picks its detector by looking at the
    # session, finds DISPLAY set by XWayland, chooses the X11 backend, fails
    # to connect and panics the worker on startup:
    #
    #   [ERROR] X11Source destruction cannot be performed, handle is null
    #   [ERROR] panicked at 'failed to initialize detector module: detection
    #           source initialization failed'
    #
    # systemd restarts it and it loops.  espanso-wayland is the same version
    # built with the wayland feature, using EVDEV for detection and injection
    # throughout, which is also why it needs the uinput access granted in
    # platform/nix/config/nixos-modules/core/uinput.nix.
    package = pkgs.espanso-wayland;
    configs = {
      default = {
        show_notifications = false;
      };
    };
    matches = {
      base = {
        matches = [
          {
            trigger = ":100";
            replace = "💯";
          }
          {
            trigger = ":nix";
            replace = "❄️";
          }
          {
            trigger = ":rust";
            replace = "🦀";
          }
          {
            trigger = ":mojo";
            replace = "🔥";
          }
          {
            trigger = ":cpp";
            replace = "💣";
          }
          {
            trigger = ":ok";
            replace = "✅";
          }
          {
            trigger = ":todo";
            replace = "🚧";
          }
          {
            trigger = ":no";
            replace = "🚫";
          }
          {
            trigger = ":eu";
            replace = "🇪🇺";
          }
          {
            trigger = ":dk";
            replace = "🇩🇰";
          }
          {
            trigger = ":us";
            replace = "🇺🇸";
          }
          {
            trigger = ":at";
            replace = "🌀";
          }
        ];
      };
    };
  };
}
