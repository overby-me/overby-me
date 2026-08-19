# Userspace OOM handling for the user session.
#
# systemd-oomd ran on every host and watched nothing: no slice carried a
# ManagedOOM* property, so `oomctl` listed zero monitored cgroups and the
# kernel's global OOM killer was the only line of defence. On 2026-08-19 a
# runaway nix evaluator (51G of a 30G machine) exercised that path: the
# kernel shot the evaluator, then systemd failed the shared launcher scope
# and took the terminal session with it.
#
# This stamps ManagedOOMMemoryPressure=kill on user.slice and every
# user-owned slice, so oomd watches PSI stall time and kills the single
# worst leaf cgroup after sustained pressure - during the thrash, not after
# allocation failure, and chosen per cgroup rather than by kernel heuristic
# plus scope-failure collateral. It is the backstop behind the per-session
# scopes zellij-cwd creates; those are what make its choice of victim
# surgical. Builds are bounded separately by nix-daemon's caps in nix.nix.
{
  systemd.oomd.enableUserSlices = true;
}
