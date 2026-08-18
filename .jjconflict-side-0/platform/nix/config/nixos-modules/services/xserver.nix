{pkgs, ...}: {
  services.xserver = {
    enable = true;
    excludePackages = [pkgs.xterm];
    xkb = {
      layout = "us";
      variant = "altgr-intl";
    };
    videoDrivers = ["amdgpu" "modesetting"];
  };
}
