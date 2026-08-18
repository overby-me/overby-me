{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    distrobox
    bubblewrap
    appimage-run
    cloud-hypervisor
    simg2img
  ];
}
