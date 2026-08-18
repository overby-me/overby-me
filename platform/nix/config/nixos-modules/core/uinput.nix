# espanso is why this exists. Under Wayland it injects expansions through
# /dev/uinput, which is root-owned 0600 by default, so the worker panics on
# startup and systemd restarts it into a loop that reads as espanso being
# broken. It blames a recent kernel update and suggests a reboot; the module is
# fine, the permissions are not.
#
# The `input` group in core/users.nix covers only reading.
{
  hardware.uinput.enable = true;

  users.users."overby.me".extraGroups = ["uinput"];
}
