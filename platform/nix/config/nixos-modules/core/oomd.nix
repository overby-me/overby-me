# Without this, oomd watched zero cgroups and the 2026-08-19 runaway nix
# evaluator went to the kernel's global OOM killer, which took the whole
# terminal session with it. PSI-based per-cgroup kills; surgical only
# because zellij-cwd scopes each session. Builds are capped in nix.nix.
{
  systemd.oomd.enableUserSlices = true;
}
