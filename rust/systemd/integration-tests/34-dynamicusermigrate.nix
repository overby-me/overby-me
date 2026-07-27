{
  name = "34-DYNAMICUSERMIGRATE";
  testTimeout = 300;
  # Was a FAKE PASS (`touch /testok` with nothing run). It carries no override
  # now and runs the real upstream script, so the result below is honest.
  #
  # PASSING: all four test_directory phases (StateDirectory, RuntimeDirectory,
  # CacheDirectory, LogsDirectory), each through DynamicUser=0, DynamicUser=1
  # and the conversion back, including the closing unit-parsing section with
  # nested and escaped-colon directory names.
  #
  # test_check_writable is NOT passing, and an earlier claim here that it was is
  # RETRACTED. It went green in exactly one VM run and does not reproduce: a
  # later run of a tree with no change to any libsystemd file fails it again.
  # One green run on an assertion this environment-sensitive proves nothing.
  # Whatever else is wrong has not been found; the ProtectSystem=strict fix
  # below was necessary but is not sufficient.
  #
  # TWO CORRECTIONS, both mine, recorded so the next reader does not repeat
  # either. The failure is that the service sees ZERO writable directories,
  # which is what this file originally said.
  #
  # I overturned that once, claiming the rationale was "inverted" and that the
  # test failed because TOO MUCH was writable. That was wrong. It rested on an
  # instrument that called access(W_OK) as ROOT at the end of
  # setup_mount_namespace, before the service drops to the DynamicUser uid.
  # Root sees a writable filesystem whatever the directory modes are; the
  # test's `find -type d -writable` runs as uid 61220 after the drop. Measuring
  # the wrong principal produced a confident and completely wrong conclusion.
  #
  # The service's own trace settles it:
  #     + [[ 0 == 8 ]]
  # and a find with upstream's exact prune list, run inside the service, prints
  # nothing at all.
  #
  # SO THE OPEN QUESTION IS WHY THE DYNAMIC USER CANNOT WRITE ITS OWN STATE
  # DIRECTORIES. They are created 61220:61220 mode 755, so the leaf modes are
  # not the problem; suspect traversal, i.e. whether /var/lib/private itself is
  # reachable by that uid inside the service's namespace. MEASURE IT, do not
  # assert it: print `id`, `stat` of /var/lib/private and of one leaf, and
  # whether the user can list them, from inside the failing service.
  #
  # The actual defect was in ProtectSystem=strict, which DynamicUser=yes
  # implies. rust-systemd restored /dev, /proc, /sys, /run, /tmp, /var/tmp and
  # /var/log to read-write after remounting / read-only. Upstream's
  # protect_system_strict_table (src/core/namespace.c:255) restores only /proc,
  # /sys and /dev, plus /home, /run/user and /root, which ProtectHome= then
  # re-protects. The extra four meant a strict service could write across the
  # whole runtime and log trees, so `find` reported far more than 8.
  #
  # REMAINING FAILURE: test_check_idmapped_mounts. The kernel here is new
  # enough (6.18 >= 5.12) that upstream's version gate lets it run. The service
  # uses MountAPIVFS=yes, DynamicUser=yes, PrivateUsers=yes and
  # TemporaryFileSystem=/run /var/opt /var/lib /vol, and logged three failures:
  #     Failed to mount tmpfs on /var/opt: No such file or directory
  #     Failed to mount tmpfs on /vol: No such file or directory
  #     Failed to create user namespace for PrivateUsers=: Operation not permitted
  #
  # The first two look easy and are NOT. Upstream mkdir -p's every mount entry's
  # path before mounting it (namespace.c:1397 and friends), so
  # TemporaryFileSystem=/vol works on a host with no /vol. Adding a bare
  # create_dir_all() before the tmpfs mount in exec_helper.rs REGRESSED
  # test_check_writable, which had been passing: that mount block is shared with
  # the private/ tmpfs, so creating missing directories there adds writable
  # directories and breaks the exact-8 assertion. TRIED AND REVERTED; a fix has
  # to create the directory only for genuine TemporaryFileSystem= entries, not
  # for every caller of that block.
  #
  # The THIRD is the real blocker and has NOT been diagnosed. Do not guess at
  # it; two plausible causes have already been narrowed:
  #   - it is NOT an ordering problem. rust sets up the mount namespace before
  #     unsharing CLONE_NEWUSER, and upstream does the same whenever it holds
  #     CAP_SYS_ADMIN (exec-invoke.c: apply_mount_namespace then
  #     setup_private_users).
  #   - upstream does FORK before unsharing: setup_private_users_child()
  #     unshares CLONE_NEWUSER in a child and the parent, still outside the new
  #     namespace, writes its /proc/<pid>/uid_map and gid_map. rust unshares
  #     in-process. That difference matters for writing the maps, but the EPERM
  #     here is on the unshare itself, so it does not explain this.
  #
  # THE ORACLE IS NO HELP HERE, already tried:
  # `c-systemd-test-34-dynamicusermigrate` fails far EARLIER than rust does, at
  # testservice-34.service in the very first test_directory phase, so upstream
  # systemd cannot get through this test in this VM at all. That makes the
  # oracle inconclusive about the PrivateUsers failure rather than evidence that
  # it is environmental. Note rust-systemd currently gets substantially further
  # through this test than the C oracle does; the C variant's own early failure
  # is worth a look on its own, since it suggests the c-systemd harness wiring
  # is incomplete for this test.
  #
  # STILL TO MEASURE: `sysctl user.max_user_namespaces` inside the VM, which
  # returns EPERM to root as well when it is 0.
  #
  # TEMPORARY diagnostic: print what the service's own find actually sees, so
  # the extra writable directories are identified rather than guessed at. The
  # same prune list as upstream is used so the output is comparable with the
  # expected set of 8.
  #
  # No `%` anywhere: this text ends up inside an ExecStart=, where `%` is
  # systemd's specifier prefix, and a `sed "s%^%PREFIX %"` there came back as
  # "unterminated `s' command". Bracket the output with markers instead of
  # prefixing each line. Prepending onto the assertion's own line avoids having
  # to reproduce its bash line-continuations.
  #
  # The `|| true` is load-bearing: the script runs under `set -e` and
  # `set -o pipefail`, and find exits non-zero when it meets an unreadable
  # directory, which killed the script before it printed anything. The test's
  # own find escapes that only because it sits inside a process substitution.
  patchScript = ''
    sed -i 's@^\(.*== "8".*\)$@    echo WRITABLE-DIAG-BEGIN >\&2; { find / \\( -path /var/tmp -o -path /tmp -o -path /proc -o -path /dev/mqueue -o -path /dev/shm -o -path /sys/fs/bpf -o -path /dev/.lxc -o -path /sys/devices/system/cpu \\) -prune -o -type d -writable -print 2>/dev/null | sort -u >\&2; } || true; echo WRITABLE-DIAG-END >\&2; \1@' TEST-34-DYNAMICUSERMIGRATE.sh
  '';

  # TOOLING NOTES:
  #   - PID 1's `log::` macros do NOT reach the console, only
  #     `crate::entrypoints::service_manager::kmsg()` does. Instrumentation
  #     added with log:: is invisible at every level, and its absence must not
  #     be read as the code path not running.
  #   - a diagnostic that reports the mount "covering" a path must handle `/`
  #     appearing twice in /proc/self/mountinfo after a bind of / onto itself,
  #     or a longest-prefix tie-break silently reports whichever entry came
  #     last.
}
