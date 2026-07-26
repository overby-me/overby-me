{
  name = "34-DYNAMICUSERMIGRATE";
  testTimeout = 300;
  # Was a FAKE PASS (`touch /testok` with nothing run). Now an honest skip that
  # gets through nearly all of the suite.
  #
  # PASSING: all four test_directory phases (StateDirectory, RuntimeDirectory,
  # CacheDirectory, LogsDirectory), each through DynamicUser=0, DynamicUser=1
  # and the conversion back, including the closing unit-parsing section with
  # nested and escaped-colon directory names. Ten exec-directory and oneshot
  # defects were fixed getting here; see docs/TEST-OVERRIDES.md and the git log.
  #
  # REMAINING FAILURE: test_check_writable. The service asserts it can see
  # exactly 8 writable directories, all under /var/lib/private/. It sees 0.
  #
  # ESTABLISHED (do not re-derive):
  #   - the directory LAYOUT is now correct: zero symlink errors and zero
  #     bind-mount errors, down from 2 and 4.
  #   - the failure is an ORDERING problem in setup_mount_namespace.
  #     ProtectSystem=strict (implied by DynamicUser=) remounts / read-only and
  #     restores the exec directories read-write, but that runs BEFORE the
  #     private/ tmpfs is mounted over <base>/private. The tmpfs discards those
  #     restored mounts, and the binds that follow inherit a read-only source.
  #
  # TRIED AND REVERTED: re-applying bind_mount_readwrite to every exec directory
  # as a final step. It made the run fail EARLIER than before, inside
  # test_directory, because `:ro` entries (`www::ro`, `www:ro:ro`) must stay
  # non-writable. A blanket re-apply cannot express that. The fix is to reorder:
  # mount the private tmpfs and its binds BEFORE the ProtectSystem pass, so the
  # existing read-only/read-write logic applies to the final mount tree.
  #
  # A failure moving BACKWARDS is the cheap signal that a change regressed; it
  # caught this one.
  #
  # THREE APPROACHES TRIED AND REVERTED for the writability problem. Do not
  # repeat them; each regressed test_directory, which had been passing:
  #   (a) blanket bind_mount_readwrite over every exec dir at the end of
  #       setup_mount_namespace. `:ro` entries (www::ro, www:ro:ro) must stay
  #       non-writable, and a blanket re-apply cannot express that.
  #   (b) moving the private/ tmpfs and its binds BEFORE the ProtectSystem=
  #       pass. TemporaryFileSystem= mounts its own tmpfs AFTER ProtectSystem,
  #       so this buried the private binds underneath it.
  #   (c) (a) again but skipping config.read_only_paths. Still regressed, so the
  #       interaction is not simply about the `:ro` set.
  #
  # The mount order is constrained from both sides: ProtectSystem=strict must
  # remount / read-only first, TemporaryFileSystem= and the private/ tmpfs mount
  # on top of that, and the binds populate them last. Whatever restores
  # writability has to survive all three without disturbing them. Worth
  # instrumenting the actual mount table (kmsg /proc/self/mountinfo) inside a
  # failing service before attempting a fourth approach.
  #
  # AFTER THAT: test_check_idmapped_mounts, gated on kernel >= 5.12.
  #
  # TOOLING NOTE: PID 1's `log::` macros do NOT reach the console, only
  # `crate::entrypoints::kmsg()` does. Instrumentation added with log:: is
  # invisible at every level, and its absence must not be read as the code path
  # not running. `testTimeout = 150;` makes the harness dump PID 1's journal in
  # ~2 minutes instead of 30.
}
