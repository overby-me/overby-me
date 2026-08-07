# /dev/uinput access, for anything that injects synthetic input events.
#
# espanso is the reason this exists.  Under Wayland it falls back to its EVDEV
# backend, which reads /dev/input/* and writes the expansion back through
# /dev/uinput.  Without this the node is root-owned and mode 0600, so the
# injector cannot open it and the worker panics on startup:
#
#   [INFO] using EVDEVInjector
#   [ERROR] Error: could not open uinput device
#   [ERROR] panicked at 'failed to initialize injector module: could not open
#           uinput device'
#
# systemd restarts it, it panics again, and the service sits in a restart loop
# that looks like espanso being broken rather than a missing permission.  Note
# the misleading hint espanso prints alongside it, blaming a recent kernel
# update and suggesting a reboot; the module is fine, the permissions are not.
#
# hardware.uinput.enable loads the module, creates the group and installs the
# udev rule; membership of that group is what actually grants access, and the
# `input` group in core/users.nix covers only the reading half.
{
  hardware.uinput.enable = true;

  users.users."overby.me".extraGroups = ["uinput"];
}
