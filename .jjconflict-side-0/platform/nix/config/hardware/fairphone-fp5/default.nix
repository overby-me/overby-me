{...}: {
  imports = [
    ./hardware.nix
    ./modem.nix
    ./bootmac.nix
  ];

  nixos-fairphone-fp5.enable = true;

  # Inject Fairphone 5 packages into pkgs via overlay so that the
  # sub-modules can reference them as plain `pkgs.<name>`.
  nixpkgs.overlays = [
    (final: _prev: {
      pil-squasher = final.callPackage ./pkgs/pil-squasher.nix {};
      firmware-fairphone-fp5 = final.callPackage ./pkgs/firmware-fairphone-fp5.nix {};
      kernel-fairphone-fp5 = final.callPackage ./pkgs/kernel-fairphone-fp5 {};
      qrtr = final.callPackage ./pkgs/qrtr.nix {};
      qmic = final.callPackage ./pkgs/qmic.nix {};
      pd-mapper = final.callPackage ./pkgs/pd-mapper {};
      rmtfs = final.callPackage ./pkgs/rmtfs.nix {};
      tqftpserv = final.callPackage ./pkgs/tqftpserv {};
      bootmac = final.callPackage ./pkgs/bootmac.nix {};
      alsa-ucm-conf-fairphone-fp5 = final.callPackage ./pkgs/alsa-ucm-conf-fairphone-fp5.nix {};
    })
  ];
}
