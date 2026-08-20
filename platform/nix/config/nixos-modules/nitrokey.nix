{config, ...}: {
  secretspec.secrets.U2F_KEYS = {
    encoding = "base64";
    mode = "0444"; # Readable by PAM
  };

  # Smart card daemon
  services.pcscd.enable = true;

  # Configure PAM U2F
  security.pam.u2f = {
    enable = true;
    control = "sufficient";
    settings = {
      cue = true;
      authfile = config.secretspec.secrets.U2F_KEYS.path;
    };
  };

  # Enable for sudo
  security.pam.services = {
    sudo.u2fAuth = true;
    login.u2fAuth = true;
  };

  # GPG and SSH
  programs.gnupg.agent = {
    enable = true;
    enableSSHSupport = true;
  };
}
