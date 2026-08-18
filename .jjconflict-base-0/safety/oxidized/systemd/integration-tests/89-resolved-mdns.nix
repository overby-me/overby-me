{
  name = "89-RESOLVED-MDNS";
  # WHERE IT STOPS, measured 2026-07-28. The mDNS half of the test needs two
  # nspawn containers. It gets 675 traced lines in, then dies at
  #
  #     machinectl start test-mdns-1
  #
  # with "Unit systemd-nspawn@test-mdns-1.service failed to start".
  #
  # ATTRIBUTION FIRST, because an earlier version of this note got it wrong.
  # Two different processes log under the same journal tag, and the tag names
  # the UNIT, not the code that emitted the line:
  #
  #   systemd-nspawn[5798]  -- the container's PID 1, which is OUR systemd
  #                            (libsystemd). Its immediately preceding lines are
  #                            libsystemd::units::loading: 208 units loaded,
  #                            pruned to 129, with host unit names such as
  #                            nix-daemon.service.
  #   systemd-nspawn[1]     -- the host-side nspawn, which logs the exec failure.
  #
  # So the container is NOT an empty rootfs: it is the host tree, via the
  # upstream drop-in's `--directory=/ --volatile=yes` plus an /etc overlay.
  # And the panic below is in libsystemd, NOT in crates/nspawn -- crates/nspawn
  # contains no thread::spawn and no thread::Builder at all.
  #
  # THE FAILURE, and what has been ruled OUT. Re-measured 2026-07-28 after two
  # fixes landed; an earlier version of this note presented an rlimit chain
  # whose first link was explicitly marked as inference. THAT INFERENCE IS NOW
  # REFUTED -- keep the refutation, it cost a VM run to get.
  #
  # The container's PID 1 (our libsystemd) still cannot create its first thread:
  #
  #   [libsystemd::entrypoints::service_manager][ERROR]
  #     Failed to spawn the signal-handler thread: Invalid argument (os error 22)
  #
  # It no longer PANICS: the five bare `std::thread::spawn` sites in
  # service_manager.rs now go through `spawn_critical_thread`, so the failure
  # names the thread and the errno instead of an anonymous unwrap inside
  # library/std. That change is what made this diagnosable; it did not fix it.
  #
  # THE EINVAL IS NOW FIXED. Root cause: we exec'd the container payload in the
  # very process that had called `unshare(CLONE_NEWPID)`. Per clone(2),
  # CLONE_THREAD is rejected with EINVAL once a process has done that, because
  # unshare moves only that process's CHILDREN into the new PID namespace, never
  # itself -- so the payload could run single-threaded (it loaded and pruned its
  # units) and then died the instant it created its first thread. It was not
  # actually PID 1 of the container either. Fixed by forking once more after the
  # namespace setup, the way upstream splits its outer and inner child: the
  # grandchild is PID 1 in the new namespace and execs the payload, while the
  # intermediate waits and propagates its exit status.
  #
  # MEASURED: the container's PID 1 now gets past unit loading and is running
  # activate_units_recursive, and no thread-spawn error appears at all.
  #
  # TWO EARLIER CANDIDATES WERE RULED OUT BY MEASUREMENT FIRST. Keep them; each
  # cost a VM run:
  #
  # 1. MISSING CONTAINER RLIMITS. crates/nspawn applied none where upstream
  #    installs a table via setrlimit_closest_all (nspawn.c:3636, table at
  #    :6007, including [RLIMIT_STACK] = { 8388608, RLIM_INFINITY }). The table
  #    is now implemented anyway, because a container inheriting the host's
  #    limits is wrong regardless -- but the EINVAL persisted unchanged with it
  #    applied, so it was NOT the cause.
  #
  # 2. THE SECCOMP FILTER. SECCOMP_DEFAULT_DENY_SYSCALLS (main.rs:555) contains
  #    no clone/clone3, and the filter answers EPERM for what it does deny,
  #    whereas thread creation was failing with EINVAL.
  #
  # WHAT STILL BLOCKS THE TEST is the fixture, which is where this note started:
  # `machinectl start test-mdns-1` spawns a container on
  # /var/lib/machines/test-mdns-1, a tree the test only populates with an /etc
  # skeleton. There is no init and no /bin/sh in it, so the exec fails with
  # ENOENT and the container exits 127. The C oracle cannot get past this
  # either.
  #
  # A THIRD DIVERGENCE, noted but NOT measured: upstream's exec of /bin/sh
  # (nspawn.c:3830-3831) is the fallback used when NOT booting a container, so
  # our nspawn reaching it at all -- rather than the --boot init path -- may be
  # wrong on its own. Do not treat that as established without measuring it.
  #
  # SEPARATELY, --volatile is accepted and then ignored: crates/nspawn parses it
  # into args.volatile (main.rs:1249-1256), merges it from .nspawn settings
  # (main.rs:2547 and 2659-2663), defaults it (main.rs:1013) and documents it in
  # --help (main.rs:3865) -- and never reads it again. Nothing consumes it to set
  # up mounts, so the flag is a silent no-op.
  #
  # Greening the test needs the nspawn work that also blocks
  # 87-aux-utils-vm-coredump, which is deep and deferred.
}
