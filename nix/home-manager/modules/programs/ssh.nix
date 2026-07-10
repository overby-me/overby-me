{
  lib,
  config,
  inputs,
  nixosConfig ? {},
  ...
}: let
  inherit (inputs.self) secrets;
  hasSecrets = nixosConfig.hasSecrets or true;
in {
  programs.ssh = {
    enable = true;
    enableDefaultConfig = false;
    settings = {
      "*" = {
        AddKeysToAgent = "yes";
        ControlMaster = "auto";
        ControlPath = "~/.ssh/socket/%r@%h:%p";
        ControlPersist = "120";
        ForwardAgent = true;
        ServerAliveInterval = 5;
        ServerAliveCountMax = 3;
      };
      localhost = {
        HostName = "localhost";
        User = config.home.username;
      };
      "home.overby.me" = {
        HostName = "home.overby.me";
        User = config.home.username;
      };
    };
  };

  home = {
    # Needed for ssh to work in buildFHSEnv and Yocto
    activation = {
      copySSHConfig = lib.hm.dag.entryAfter ["linkGeneration"] ''
        configPath="$HOME/.ssh/config"
        if [ -L "$configPath" ]; then
          target="$(readlink "$configPath")"
          run rm "$configPath"
          run cp "$target" "$configPath"
          run chmod 600 "$configPath"
        fi
      '';
    };

    file =
      {
        # Forced because copySSHConfig below replaces the symlink with a real copy.
        ".ssh/config".force = true;
      }
      // lib.optionalAttrs hasSecrets (let
        inherit (secrets) publicKeys;
      in {
        ".ssh/id_ed25519.pub".text = publicKeys.overby-me-ssh-ed25519;
        ".ssh/id_rsa.pub".text = publicKeys.overby-me-ssh-rsa;
      });
  };
}
