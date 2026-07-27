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
  # SOLVED, by measuring per-bind instead of guessing a seventh time. The
  # aggregate "all three exec dirs fail" was a misreading: logging each bind's
  # source, dest and errno separately showed
  #     IDMAP ok   src=/var/lib/private/testidmapped dest=/var/lib/private/testidmapped
  #     IDMAP FAIL src=/var/lib/private/testidmapped dest=/var/lib/private/testidmapped  EPERM
  #     IDMAP FAIL src=/var/lib/private/testidmapped dest=/var/lib/sampleservice          EPERM
  #     IDMAP FAIL src=/var/lib/private/testidmapped dest=/var/lib/testidmapped           EPERM
  # ALL FOUR BINDS SHARE ONE SOURCE, and the first one succeeds. The idmap
  # belongs to the SOURCE mount, so once it is applied the next
  # open_tree(OPEN_TREE_CLONE) clones an already-idmapped mount and the kernel
  # refuses that with EPERM (is_idmapped_mnt). Each source is now idmapped
  # exactly once and the aliases are plain binds, which inherit the mapping.
  #
  # Six hypotheses died before this one, all recorded below so they are not
  # retried. What actually distinguished them was per-bind instrumentation, not
  # more reasoning about capabilities and namespaces.
  #
  # THE FAILURE HAS MOVED, which is what says the diagnosis was right:
  #     touch: cannot touch '/var/lib/sampleservice/testfile':
  #         Value too large for defined data type      (EOVERFLOW, was EPERM)
  # THE EOVERFLOW IS FIXED. `touch /var/lib/sampleservice/testfile` succeeds:
  # id-mapped exec directories are now writable by the service.
  #
  # It needed TWO changes together, which is why every single-change experiment
  # failed and produced a contradictory-looking record:
  #   - the MOUNT idmap namespace maps "<uid> 0 1". The kernel resolves a file's
  #     on-disk id by looking it up on the INSIDE of that namespace, so the
  #     on-disk owner belongs there and the id the service should see goes
  #     outside. Upstream's make_userns() emits the same shape for
  #     REMOUNT_IDMAPPING_HOST_OWNER: "<source_owner> <uid_shift> 1".
  #   - the SERVICE gets its own namespace, mapping "0 <uid> 1", and no longer
  #     joins the mount's. The two maps are opposites, so they cannot be one
  #     namespace: joining the mount's would give the process a mapping in which
  #     in-namespace 61221 is host root.
  #
  # HONEST RECORD OF HOW THIS WENT, since the middle of it is misleading on its
  # own. "The map is inverted" was proposed, then RETRACTED on the reasoning
  # that an inverted map would resolve to some specific wrong id rather than to
  # nobody. That reasoning was wrong: with "0 <uid> 1" the on-disk id has no
  # entry on the side the kernel looks up, so nobody (65534) is exactly what an
  # inverted map yields. The retraction was the error, not the original idea.
  # Separating the namespaces was also tested alone and appeared to fail,
  # because the direction was still wrong. Neither half works without the other.
  #
  # The pattern across all of it: every hypothesis reached by reasoning about
  # kernel mapping semantics died, and the things that moved the bug were
  # measurements — per-bind instrumentation for the EPERM, and stat() from the
  # correct namespace for the EOVERFLOW.
  #
  # PRIVATEUSERS= WAS ALSO WRONG, found by chasing that empty value. Upstream's
  # PRIVATE_USERS_SELF identity-maps TWO ids so both are representable inside:
  #     <saved_uid> <saved_uid> 1
  #     <uid>       <uid>       1
  # rust wrote a single "0 0 1". That one line caused two separate symptoms: the
  # empty `awk NR==2 /proc/self/uid_map` here, and the EINVAL on dropping to the
  # service's uid that was earlier worked around by dropping to (0, 0) instead.
  # The workaround is removed: with the map correct it would leave the service
  # running as namespace root rather than its own identity. The service's own
  # assertion now passes.
  #
  # WHERE IT STOPS NOW, and it is no longer about ids at all: the OUTER, host-side
  #     [[ $(stat -c "%u" /var/lib/private/testidmapped/testfile) == 65534 ]]
  # gets an empty value, i.e. the host cannot find the file the service created.
  # The service wrote /var/lib/sampleservice/testfile successfully.
  # StateDirectory=testidmapped:sampleservice means the second name is an ALIAS
  # of the first, so both must resolve to one directory.
  #
  # RULED OUT, measured: the bind source is NOT the empty tmpfs that
  # TemporaryFileSystem= lays over /var/lib. statfs()ing each source at bind
  # time against TMPFS_MAGIC reports nothing, so the binds take the real
  # filesystem and a service's writes should reach the host. That was the
  # obvious explanation for "write succeeds, host sees nothing" and it is wrong.
  #
  # STILL UNEXPLAINED, and worth stating plainly rather than guessing a twelfth
  # time: the service creates /var/lib/sampleservice/testfile successfully, its
  # own in-namespace assertion passes, the bind source is real, and yet the host
  # cannot stat /var/lib/private/testidmapped/testfile. The next measurement
  # should establish WHERE the file actually landed — e.g. have PID 1 walk the
  # real state directory after the service exits, or compare st_dev/st_ino of
  # the source before the bind with the file's after — rather than proposing
  # another mechanism. Every mechanism proposed for this test so far has died;
  # the two that ever held up came from measuring.
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
