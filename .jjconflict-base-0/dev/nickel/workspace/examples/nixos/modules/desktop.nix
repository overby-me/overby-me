# Desktop environment NixOS module
#
# This module provides a basic desktop environment configuration.
# It is auto-discovered by nix-workspace from the modules/ directory
# and referenced by name ("desktop") in machine configurations.
#
# In a real workspace, this would configure your preferred desktop
# environment, display manager, and common GUI applications.
{pkgs, ...}: {
  services = {
    # Enable X11 windowing system
    xserver = {
      enable = true;

      # Display manager
      displayManager.gdm.enable = true;

      # Desktop environment
      desktopManager.gnome.enable = true;

      # Keyboard layout
      xkb = {
        layout = "us";
        variant = "";
      };
    };

    # Disable PulseAudio (using PipeWire instead)
    pulseaudio.enable = false;

    # Enable sound with PipeWire
    pipewire = {
      enable = true;
      alsa.enable = true;
      alsa.support32Bit = true;
      pulse.enable = true;
    };

    # Enable printing support
    printing.enable = true;
  };

  # Enable RealtimeKit for PipeWire
  security.rtkit.enable = true;

  # Common desktop packages
  environment.systemPackages = with pkgs; [
    firefox
    kitty
    git
    vim
    htop
  ];

  # Enable networking tools commonly needed on desktops
  networking.networkmanager.enable = true;

  # Fonts
  fonts.packages = with pkgs; [
    noto-fonts
    noto-fonts-cjk-sans
    noto-fonts-emoji
    liberation_ttf
    fira-code
    fira-code-symbols
  ];
}
