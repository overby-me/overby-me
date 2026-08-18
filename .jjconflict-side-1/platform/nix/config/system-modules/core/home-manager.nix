# Activate a standalone home-manager configuration from `system-manager switch`,
# so one deploy provisions both the system and the user layer on a non-NixOS
# host.
#
# home-manager's own NixOS module cannot be used: it drives activation off
# `config.users.users.<name>`, and system-manager does not manage users. So this
# runs the `activate` script of an already-built standalone configuration from a
# oneshot service. The standalone config stays the single source of truth; this
# only changes who triggers it.
#
# Pick one owner: activating through this module *and* running `home-manager
# switch` for the same user makes the two fight over one generation.
#
# Does nothing until the consumer sets `home-manager.standalone.configuration`,
# so it is safe in a base config.
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

    # A login shell so PATH and locale are sane, and to pick up the user's live
    # session env if they are logged in. Mirrors home-manager's own unit.
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
