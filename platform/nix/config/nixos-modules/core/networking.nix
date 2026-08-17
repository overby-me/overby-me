{lib, ...}: {
  networking = {
    hostName = lib.mkDefault "gravitas";
    networkmanager = {
      enable = true;
      dns = "systemd-resolved";
    };
  };
}
