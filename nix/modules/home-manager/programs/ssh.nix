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
      # Trigger agenix decryption on first SSH use rather than at login.
      # If the decrypted key doesn't exist yet, start the agenix user service
      # (which prompts for Nitrokey touch) and wait before connecting.
      "agenix-trigger" = lib.mkIf hasSecrets {
        match = ''exec "if not ('%d/.ssh/id_ed25519' | path exists) { systemctl --user start agenix }"'';
      };
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

  age = lib.mkIf hasSecrets {
    identityPaths = ["${config.home.homeDirectory}/.age/id_fido2"];
    secrets = {
      id_ed25519 = {
        file = secrets.id_ed25519;
        path = "${config.home.homeDirectory}/.ssh/id_ed25519";
        mode = "600";
      };
      id_rsa = {
        file = secrets.id_rsa;
        path = "${config.home.homeDirectory}/.ssh/id_rsa";
        mode = "600";
      };
    };
  };
}
