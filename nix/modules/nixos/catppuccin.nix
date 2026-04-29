_: {
  catppuccin = {
    enable = true;
    # systemd-boot has no theming support, so disable the GRUB theme
    # that `enable = true` would otherwise activate.
    grub.enable = false;
  };
}
