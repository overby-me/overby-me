# The login sessions cosmic-greeter lists for Stardust XR: one through monado
# on the TTY, one nested in cage for when the glasses are not plugged in.
#
# Which one the server gives is chosen by environment rather than by flag: it
# adds bevy's winit plugin only when DISPLAY or WAYLAND_DISPLAY is non-empty,
# and otherwise stays headless and drives OpenXR.
{
  lib,
  cage,
  runCommand,
  writeShellApplication,
  writeText,
  stardust-xr-server,
  stardust-xr-flatland,
  stardust-xr-protostar,
  stardust-xr-non-spatial-input,
  systemd,
  xkeyboard_config,
}: let
  inherit (lib) getExe getExe';

  input = getExe' stardust-xr-non-spatial-input;

  # greetd does not capture a session's stderr, so a server that dies takes its
  # reason with it: the wgpu abort behind the first failed login was only
  # recoverable from a core dump. Cosmic's own autologin does the same thing.
  logged = "${getExe' systemd "systemd-cat"} -t stardust-xr";

  # xkbcommon looks in /usr/share/X11/xkb and finds nothing here, so without
  # this the server comes up with no keymap and nothing typed reaches a client.
  # Exported rather than passed, because eclipse builds a keymap of its own.
  # RUST_BACKTRACE joins it because these sessions are still being brought up.
  sessionEnv = ''
    export XKB_CONFIG_ROOT=${xkeyboard_config}/share/X11/xkb
    export RUST_BACKTRACE=1
  '';

  # `--execute-startup-script` runs this once the server accepts clients.
  # Backgrounded rather than waited on: logind kills the session's cgroup at
  # logout, so a wait would only hold the server open behind them. flatland is
  # what 2D applications draw into, so the launcher needs it to launch anything.
  #
  # atmosphere is deliberately not here: it is a CLI over a directory of
  # installed environments, and `atmosphere show` panics on an unwrap in
  # env.rs when that directory does not exist. Run `atmosphere install <path>`
  # first, then it belongs in this list.
  startup = name: pipeline:
    writeShellApplication {
      name = "stardust-xr-startup-${name}";
      text = ''
        ${getExe stardust-xr-flatland} &
        ${getExe' stardust-xr-protostar "hexagon_launcher"} &
        ${pipeline} &
      '';
    };

  # Upstream's idiom: an input source piped into simular, which directs it at
  # whatever window is being looked at. eclipse reads libinput on seat0, which
  # is what a session with no compositor needs; manifold is a window you focus,
  # so it only means anything under cage.
  xrSession = writeShellApplication {
    name = "stardust-xr-session";
    text = ''
      ${sessionEnv}
      # Inherited from the greeter, either would force flatscreen mode.
      unset DISPLAY WAYLAND_DISPLAY
      exec ${logged} ${getExe stardust-xr-server} --xr-only \
        --execute-startup-script ${getExe (startup "xr" "${input "eclipse"} | ${input "simular"}")}
    '';
  };

  flatscreenSession = writeShellApplication {
    name = "stardust-xr-flatscreen-session";
    text = ''
      ${sessionEnv}
      exec ${logged} ${getExe cage} -- ${getExe stardust-xr-server} --force-flatscreen \
        --execute-startup-script ${getExe (startup "flatscreen" "${input "manifold"} | ${input "simular"}")}
    '';
  };

  # The attribute name is the session name: what the greeter lists, what the
  # desktop file must be called, and what `providedSessions` repeats back.
  sessions = {
    stardust-xr = {
      label = "Stardust XR";
      comment = "Spatial desktop on the headset, through Monado";
      session = xrSession;
    };
    stardust-xr-flatscreen = {
      label = "Stardust XR (flatscreen)";
      comment = "Spatial desktop in a window, driven by mouse and keyboard";
      session = flatscreenSession;
    };
  };

  entry = name: {
    label,
    comment,
    session,
  }:
    writeText "${name}.desktop" ''
      [Desktop Entry]
      Name=${label}
      Comment=${comment}
      Exec=${getExe session}
      Type=Application
      DesktopNames=StardustXR
    '';
in
  runCommand "stardust-xr-session" {
    passthru.providedSessions = lib.attrNames sessions;

    meta = {
      description = "Login sessions for the Stardust XR display server";
      homepage = "https://stardustxr.org/";
      mainProgram = "stardust-xr-session";
      platforms = lib.platforms.linux;
    };
  } ''
    mkdir -p $out/bin $out/share/wayland-sessions

    ${lib.concatLines (lib.mapAttrsToList (name: cfg: ''
        ln -s ${getExe cfg.session} $out/bin/
        ln -s ${entry name cfg} $out/share/wayland-sessions/${name}.desktop
      '')
      sessions)}
  ''
