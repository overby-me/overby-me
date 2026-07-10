# Activate a standalone home-manager configuration as part of a single
# `system-manager switch`, so one deploy provisions both the system layer and
# the user layer on a non-NixOS host.
#
# Why a systemd service instead of home-manager's NixOS module
# -----------------------------------------------------------
# home-manager ships `home-manager.nixosModules.home-manager`, but it drives
# activation off `config.users.users.<name>` (username, home directory, uid) and
# NixOS-managed systemd services. system-manager deliberately does *not* manage
# users (`users.users` is a no-op in its module set), so that NixOS module can't
# be used here.
#
# Instead we reuse an already-built standalone `homeConfiguration` — the exact
# config you'd otherwise apply with `home-manager switch --flake .#<name>` — and
# run its generated `activate` script from a oneshot systemd service. There is a
# single source of truth for the user environment (the standalone config); this
# module just changes *who* triggers activation. The unit mirrors home-manager's
# own NixOS activation service: it runs as the user, waits for the home directory
# to be mounted, and activates through a login shell so the environment is sane.
#
# Pick one owner for the user layer: if you activate home-manager via this
# module, do not also run `home-manager switch` for the same user, or the two
# will fight over the same generation/profile.
#
# This module is generic: the consumer picks *which* home configuration to
# activate by setting `home-manager.standalone.configuration` to a built
# `homeManagerConfiguration` (e.g.
# `inputs.self.homeConfigurations.<name>`). When it is left null this module
# does nothing, so it is safe to include in a base config.
{
  pkgs,
  lib,
  config,
  ...
}: let
  cfg = config.home-manager.standalone;
in {
  options.home-manager.standalone.configuration = lib.mkOption {
    type = lib.types.nullOr lib.types.raw;
    default = null;
    description = ''
      A built standalone home-manager configuration (the result of
      `home-manager.lib.homeManagerConfiguration`, i.e. an entry of
      `homeConfigurations`) to activate on `system-manager switch`.

      Leave null to disable single-deploy home-manager activation.
    '';
  };

  config = lib.mkIf (cfg.configuration != null) (let
    inherit (cfg.configuration.config.home) username homeDirectory activationPackage;

    unitName = "home-manager-${username}";

    # Activate through a login shell so PATH, locale, etc. are sane, and pull in
    # the user's live session env if they happen to be logged in (mirrors
    # home-manager's own NixOS activation service).
    setupEnv = pkgs.writeShellScript "hm-setup-env" ''
      #! ${pkgs.runtimeShell} -el
      exec "${activationPackage}/activate"
    '';
  in {
    systemd.services.${unitName} = {
      description = "Home Manager environment for ${username}";
      # `multi-user.target` is remapped by system-manager to
      # `system-manager.target`, so this runs on `system-manager switch`.
      wantedBy = ["multi-user.target"];
      wants = ["nix-daemon.socket"];
      after = ["nix-daemon.socket"];

      # Don't activate until the user's home directory is available.
      unitConfig.RequiresMountsFor = homeDirectory;

      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        TimeoutStartSec = "5m";
        User = username;
        SyslogIdentifier = "hm-activate-${username}";
        ExecStart = "${setupEnv}";
      };

      environment.QT_QPA_PLATFORM = "offscreen";
    };
  });
}
