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
    matchBlocks = {
      "*" = {
        addKeysToAgent = "yes";
        controlMaster = "auto";
        controlPath = "~/.ssh/socket/%r@%h:%p";
        controlPersist = "120";
        forwardAgent = true;
        serverAliveInterval = 5;
        serverAliveCountMax = 3;
      };
      localhost = {
        hostname = "localhost";
        user = config.home.username;
      };
      "home.overby.me" = {
        hostname = "home.overby.me";
        user = config.home.username;
      };
    };
  };
  # Needed for ssh to work in buildFHSEnv
  home.activation = {
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

  home.file = lib.mkIf hasSecrets (let
    inherit (secrets) publicKeys;
  in {
    ".ssh/id_ed25519.pub".text = publicKeys.noverby-ssh-ed25519;
    ".ssh/id_rsa.pub".text = publicKeys.noverby-ssh-rsa;
  });
}
