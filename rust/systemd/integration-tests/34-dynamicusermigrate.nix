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
  # test_check_writable now PASSES, verified on two independent VM runs (the
  # derivation hashes differ, so neither was served from cache), alongside
  # 01-basic, 44-log-namespace and 26-systemctl to check the ProtectSystem=
  # path it touches. The measurement inside the service reads:
  #     root-mount-lines: 1
  #     writable-dirs: the eight /var/lib/private entries and nothing else
  #
  # ROOT CAUSE, measured in a single process inside the failing service.
  # It is not ownership and not traversal: the service runs as uid 61220,
  # /var/lib/private is 755 root:root, /var/lib/private/waldo is 755
  # 61220:61220, and both `touch` and `test -w` on it SUCCEED. Yet the same
  # process's `find -type d -writable` returns nothing.
  #
  # /proc/self/mountinfo explains it. THE ROOT FILESYSTEM IS MOUNTED TWICE,
  # stacked, with contradictory flags:
  #     603 602 253:0 / /  ro,relatime - ext4 /dev/vda
  #     626 603 253:0 / /  rw,relatime - ext4 /dev/vda
  # and the subtree below is duplicated with it (627-660 repeat 604-621, and
  # 690-700 repeat 679-689). The exec-directory binds hang off the READ-ONLY
  # root at 603, while the read-write root at 626 shadows it, so resolving the
  # same path can reach either. `touch` succeeds through one view and
  # access(W_OK), which is what `find -writable` calls, fails through the other.
  #
  # The duplicate root came from remount_read_only() binding every path onto
  # itself before remounting read-only. For a path that is ALREADY a mount
  # point that stacks a second mount over the first. FIXED by skipping the bind
  # when /proc/self/mountinfo already lists the path; the bind stays for paths
  # like /usr that may not be separate mounts.
  #
  # This also explains why the earlier ProtectSystem=strict fix was necessary
  # but not sufficient: it corrected WHICH paths get restored read-write, but
  # not the duplicated tree they are applied to.
  #
  # A SEPARATE ProtectSystem=strict fix also landed and stays, though it was
  # NOT the cause here: rust restored /dev, /proc, /sys, /run, /tmp, /var/tmp
  # and /var/log to read-write, where upstream's protect_system_strict_table
  # (src/core/namespace.c:255) restores only /proc, /sys and /dev plus /home,
  # /run/user and /root, which ProtectHome= then re-protects. The extra four let
  # any strict service write across the runtime and log trees.
  #
  # REMAINING FAILURE: test_check_idmapped_mounts, and it is a MISSING FEATURE
  # rather than a defect. Three real bugs were fixed out of this phase first,
  # and the failure moved each time:
  #   1. the doubly-mounted root, which also made unshare(CLONE_NEWUSER) return
  #      EPERM (man 2 unshare refuses it when the caller's root does not match
  #      its mount namespace root);
  #   2. the privilege drop targeting the OUTSIDE uid/gid inside a
  #      PrivateUsers= namespace, where the default map is "0 <uid> 1" and the
  #      only representable id is 0, giving EINVAL from setgid;
  #   3. TemporaryFileSystem= mount points being created AFTER ProtectSystem=
  #      had remounted / read-only, so the mkdir failed EROFS and the mount
  #      then failed ENOENT.
  #
  # ID-MAPPED MOUNT GROUNDWORK IS IN, BUT NOT WORKING YET. exec_helper.rs now
  # has create_mapped_userns() and idmapped_bind(). Established by labelling
  # each syscall separately, which should have been done first:
  #     open_tree(OPEN_TREE_CLONE)  -> succeeds, detached mount created
  #     mount_setattr(MOUNT_ATTR_IDMAP) -> EPERM
  #     move_mount                  -> not reached
  # So the sequence is right and the attribute itself is refused. The kernel's
  # can_idmap_mount() denies with EPERM in only two cases, and EVERY other
  # rejection there returns EINVAL. Both EPERM cases have now been measured at
  # the failure point:
  #     CapEff: 000001ffffffffff   (every capability, CAP_SYS_ADMIN included)
  #     userns: user:[4026531837]  (the INITIAL user namespace)
  # so ns_capable(fs_userns, CAP_SYS_ADMIN) cannot be what fails. That leaves
  # is_idmapped_mnt(), i.e. the kernel thinks the mount is ALREADY idmapped.
  # Worth checking whether the self-bind or the clone is inheriting an idmap,
  # or whether the same source is being processed more than once: all three
  # exec dirs fail, and two of them (/var/lib/testidmapped and
  # /var/lib/sampleservice) are aliases of the same underlying directory.
  #
  # RULED OUT BY READING THE CODE, 2026-07-27, so nobody re-tests these:
  #   - "the idmap is attempted from inside the service's user namespace, so
  #     ns_capable(sb->s_user_ns, CAP_SYS_ADMIN) fails". NO: setup_mount_namespace()
  #     is called at exec_helper.rs:2557/2565, BEFORE the setns/unshare block at
  #     2745, so idmapped_bind() runs while still in the initial user namespace.
  #     That also matches the measured user:[4026531837].
  #   - "the userns fd passed to mount_setattr is the INITIAL namespace, which
  #     the kernel rejects with EPERM by design". NO: create_mapped_userns()
  #     checks the forked child's unshare(CLONE_NEWUSER) actually succeeded
  #     before writing uid_map/gid_map and opening /proc/<pid>/ns/user, so the
  #     fd is a genuine child namespace.
  #
  # STILL UNTESTED, and the one the earlier notes flagged: whether the same
  # underlying directory is idmapped twice. /var/lib/testidmapped and
  # /var/lib/sampleservice are aliases of one directory, so a second
  # open_tree(OPEN_TREE_CLONE) could clone a mount the first pass already
  # idmapped and moved into place, which is_idmapped_mnt() refuses with EPERM.
  # Against that: the earlier run reported ALL THREE exec dirs failing, and this
  # hypothesis predicts the first one succeeding. THE MEASUREMENT THAT SETTLES
  # IT is to log, per bind, the source path and the exact errno, rather than a
  # single aggregate "it failed" - that distinguishes "first succeeds, rest
  # EPERM" from "all EPERM" and is worth one VM run.
  #
  # THREE WRONG GUESSES were made before that, all about kernel rules, each
  # costing a VM run: that an already-attached mount could be idmapped (it
  # cannot, it must be detached, EINVAL); that open_tree on a bare directory
  # clones that directory (it clones the CONTAINING mount, so a self-bind is
  # needed first); and that shared propagation was the blocker (making it
  # MS_PRIVATE changed nothing). Label your syscalls before theorising.
  #
  # The groundwork is SAFE to leave in place: the plain bind still happens in
  # Step 3 and the idmap is an overlay attempted afterwards, so a failure only
  # logs. test_check_writable still passes with it in.
  #
  # What is left needs ID-MAPPED MOUNTS:
  #     touch: cannot touch '/var/lib/sampleservice/testfile': Permission denied
  # The service runs as in-namespace uid 0, which maps to outside uid 61220,
  # while its StateDirectory is owned by outside 61220. That uid is not mapped
  # inside the namespace, so the directory appears owned by nobody and is
  # unwritable. Upstream attaches the exec directories with an id-mapped mount
  # (mount_setattr with MOUNT_ATTR_IDMAP and a userns fd) so the ownership is
  # translated. rust-systemd has no mount_setattr/MOUNT_ATTR_IDMAP support at
  # all, and that is what the test is named for.
  #
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
