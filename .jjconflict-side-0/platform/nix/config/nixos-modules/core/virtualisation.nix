{
  virtualisation = {
    docker = {
      enable = true;
      daemon.settings = {
        # runtimes = {
        #   youki = {
        #     path = "${pkgs.youki}/bin/youki";
        #   };
        # };
        # default-runtime = "youki";
      };
    };
    libvirtd.enable = true;
    waydroid.enable = true;
  };
}
