# System-level packages installed into the system profile.
#
# These land on every user's PATH via /etc/profile.d/system-manager-path.sh
# (system-manager writes this on first activation; log out/in or source it to
# pick it up). Prefer this for machine-wide tooling; per-user tools belong in a
# home-manager configuration instead.
{pkgs, ...}: {
  environment.systemPackages = with pkgs; [
    git
    curl
    htop
  ];
}
