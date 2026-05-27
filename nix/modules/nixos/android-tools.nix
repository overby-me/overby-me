{pkgs, ...}: {
  environment.systemPackages = [pkgs.android-tools];
  users.users."overby.me".extraGroups = ["adbusers" "dialout"];
  boot.kernel.sysctl."kernel.dmesg_restrict" = 0;
}
