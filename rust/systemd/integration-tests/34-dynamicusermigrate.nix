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
  # THE FAILURE IS NOW EOVERFLOW, and two further hypotheses about it are dead.
  # Measured FROM INSIDE the service's own namespace (the earlier probe sat in
  # setup_mount_namespace(), which runs BEFORE the userns join at
  # exec_helper.rs:2745, so it was reading from the initial namespace and could
  # say nothing about what the service sees — the same "measured the wrong
  # principal" mistake as the access(W_OK)-as-root one above):
  #     IDMAP in-ns /var/lib/private/testidmapped st_uid=65534 st_gid=65534
  #     IDMAP in-ns /var/lib/sampleservice        st_uid=65534 st_gid=65534
  #     IDMAP in-ns /var/lib/testidmapped         st_uid=65534 st_gid=65534
  # for an on-disk owner of 61221. 65534 is nobody, i.e. UNMAPPED rather than
  # mapped to the wrong id.
  #
  # DEAD HYPOTHESIS 7, "the map is inverted": an inverted map would resolve to
  # some other specific id, not to nobody. A claim that it was inverted was
  # briefly recorded here and in commit b664a76a; it is retracted.
  #
  # DEAD HYPOTHESIS 8, "the mapping is applied twice because the service joins
  # the very namespace the mount was idmapped against": tested by giving the
  # service its own namespace instead (`let joined = false`). The writes still
  # fail with EOVERFLOW, so that is not it either, and the change was reverted
  # since nothing supports it and it alters PrivateUsers= semantics.
  #
  # NOTE ON THAT RUN: the in-ns probe lived inside the `if joined` arm, so
  # setting joined=false silently removed the very measurement being relied on.
  # The probe is unconditional now. Put diagnostics OUTSIDE the branch whose
  # behaviour is under test.
  #
  # WHAT IS STILL UNKNOWN: why an on-disk id of 61221, with a mount idmap whose
  # uid_map is "0 61221 1", presents as unmapped. Worth checking next, with a
  # measurement rather than another theory: whether the mount actually carries
  # the idmap after move_mount (read /proc/self/mountinfo for the dest and look
  # for the idmap field), and what upstream's make_userns() writes for this case
  # (src/shared/mount-util.c:1519, REMOUNT_IDMAPPING_HOST_OWNER emits
  # "<source_owner> <uid_shift> 1").
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
