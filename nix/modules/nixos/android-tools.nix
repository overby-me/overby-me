{pkgs, ...}: {
  environment.systemPackages = [pkgs.android-tools];
  users.users.noverby.extraGroups = ["adbusers" "dialout"];
  boot.kernel.sysctl."kernel.dmesg_restrict" = 0;
}
