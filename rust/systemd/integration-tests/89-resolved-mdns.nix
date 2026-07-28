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
  # THE PANIC, and the chain that leads to it. Links 2 and 3 are observed;
  # link 1 -> 2 is inference, not proof.
  #
  # 1. crates/nspawn applies NO rlimits to the container payload (grep for
  #    RLIMIT/setrlimit/rlimit under crates/nspawn/src returns nothing), so the
  #    payload inherits the host's. Upstream deliberately resets the container
  #    table, including
  #
  #      [RLIMIT_STACK] = { 8388608, RLIM_INFINITY }   (nspawn.c:6007)
  #
  #    applied via setrlimit_closest_all (nspawn.c:3636).
  #
  # 2. The container's PID 1 then hits pthread_create EINVAL:
  #
  #      thread 'main' panicked at library/std/src/thread/functions.rs
  #      failed to spawn thread: Os { code: 22, kind: InvalidInput }
  #
  #    which is the signature of an unusable thread stack size.
  #
  # 3. It PANICS rather than erroring because the four core PID 1 threads use
  #    bare `std::thread::spawn`, whose failure path is an unwrap:
  #    start_notification_handler_thread / stdout / stderr / signal, at
  #    crates/libsystemd/src/entrypoints/service_manager.rs:1286, 1291, 1296
  #    and 1309. Sibling spawn sites already do this correctly, with
  #    thread::Builder plus explicit error handling: watchdog.rs:68,
  #    timer_scheduler.rs:205, service_manager.rs:256, path_watcher.rs:428,
  #    dbus_server.rs:1570 and activate.rs:2040.
  #
  # A panic is never correct behaviour, and mirroring the Builder sites is the
  # small self-contained fix here. The missing container rlimit table is a
  # separate, genuine divergence from upstream.
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
