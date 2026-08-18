{
  config,
  inputs,
  ...
}: {
  age.secrets.u2f-keys = {
    file = inputs.self.secrets.u2f-keys;
    path = "/run/agenix/u2f-keys";
    mode = "0444"; # Readable by PAM
    owner = "root";
    group = "root";
  };

  # Smart card daemon
  services.pcscd.enable = true;

  # Configure PAM U2F
  security.pam.u2f = {
    enable = true;
    control = "sufficient";
    settings = {
      cue = true;
      authfile = config.age.secrets.u2f-keys.path;
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
