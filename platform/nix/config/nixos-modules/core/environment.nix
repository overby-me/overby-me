{pkgs, ...}: {
  environment = {
    systemPackages = with pkgs; [
      helix
      tailspin
    ];
    sessionVariables = {
      PAGER = "${pkgs.tailspin}/bin/tspin";
      SYSTEMD_PAGERSECURE = "1";
      NIXOS_OZONE_WL = "1";
    };
  };
}
