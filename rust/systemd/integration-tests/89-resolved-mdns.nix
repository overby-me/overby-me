{
  name = "89-RESOLVED-MDNS";
  # WHERE IT STOPS, measured 2026-07-28. The mDNS half of the test needs two
  # nspawn containers. It gets 675 traced lines in, then dies at
  #
  #     machinectl start test-mdns-1
  #
  # with "Unit systemd-nspawn@test-mdns-1.service failed to start". Two separate
  # problems are visible in the journal, and only one of them is about mDNS.
  #
  # 1. FIXTURE. The container tree the test builds under
  #    /var/lib/machines/test-mdns-1 has no shell, so nspawn reports
  #    "exec(/bin/sh) failed: No such file or directory" and the container exits
  #    127. NixOS has no /bin/sh, and nothing populates one here.
  #
  # 2. A REAL DEFECT, worth fixing on its own. Just before that, our
  #    systemd-nspawn PANICS:
  #
  #      thread 'main' panicked at library/std/src/thread/functions.rs
  #      failed to spawn thread: Os { code: 22, kind: InvalidInput }
  #
  #    A failed thread spawn should be an error, not a panic. That is a bug in
  #    crates/nspawn regardless of whether the container rootfs is usable, and
  #    it would be worth fixing even though the surrounding nspawn work is
  #    deep and deferred.
  #
  # Greening the test needs container rootfs provisioning as well, which is the
  # same nspawn area that blocks 87-aux-utils-vm-coredump.
}
