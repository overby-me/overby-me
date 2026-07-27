{
  name = "67-INTEGRITY";
  testTimeout = 300;
  # GREEN as of 2026-07-27, running the real upstream test with no override:
  # all ten test_one cases pass (crc32c, crc32, xxhash64, sha1, sha256, each
  # with the integrity metadata inline and on a separate data device).
  #
  # The one thing this test needs from the harness is integritysetup, the
  # cryptsetup project's CLI, which the test drives to create the dm-integrity
  # device before systemd-integritysetup@ attaches it. It is a test dependency,
  # not a rust-systemd gap.
  #
  # Two defects had to be fixed to get here, neither of them in
  # crates/integritysetup, which was correct all along:
  #   - udev kept the quotes on properties imported from a program, so
  #     `dmsetup udevflags` emitting DM_UDEV_PRIMARY_SOURCE_FLAG='1' never
  #     matched 10-dm.rules' =="1". The rules concluded the event was not from
  #     the primary source, set DM_UDEV_DISABLE_DISK_RULES_FLAG, and
  #     13-dm-disk.rules skipped the device, so a dm device with a filesystem
  #     on it never got its /dev/disk/by-uuid/ symlink and `blkid -U` resolved
  #     the filesystem to the underlying loop device instead.
  #   - device symlinks were last-writer-wins, so even once both devices
  #     claimed /dev/disk/by-uuid/<uuid> the winner was arbitrary.
  #     OPTIONS="link_priority=" now decides it, which is what this test's own
  #     99-priority.rules relies on.
  #
  # Worth recording for the next investigation, because three separate
  # rationales in this file were wrong before the right one turned up:
  #   - an earlier revision claimed extraPackages already supplied cryptsetup
  #     while the attribute was absent, so six commits' worth of findings came
  #     from an uncommitted working copy and were void.
  #   - `systemctl is-active` returning 3 while printing `inactive` matches
  #     upstream and is not a bug.
  #   - a diagnostic ERR trap must write to stderr: it also fires inside a
  #     $( ), where anything on stdout is captured into the caller's variable
  #     and corrupts the comparison being diagnosed.
  extraPackages = pkgs: [ pkgs.cryptsetup ];
}
