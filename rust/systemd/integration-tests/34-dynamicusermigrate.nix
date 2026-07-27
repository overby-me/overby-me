{
  name = "34-DYNAMICUSERMIGRATE";
  testTimeout = 300;
  # Was a FAKE PASS (`touch /testok` with nothing run). It carries no override
  # now and runs the real upstream script, so the result below is honest.
  #
  # PASSING: all four test_directory phases (StateDirectory, RuntimeDirectory,
  # CacheDirectory, LogsDirectory), each through DynamicUser=0, DynamicUser=1
  # and the conversion back, including the closing unit-parsing section with
  # nested and escaped-colon directory names; and test_check_writable.
  #
  # A CORRECTION WORTH READING BEFORE TOUCHING THIS TEST. Earlier revisions of
  # this file recorded test_check_writable as failing because the exec
  # directories "end up read-only ... It sees 0" writable directories, and
  # three separate approaches were tried and reverted trying to make them
  # writable. That rationale was INVERTED. Instrumenting the mount table showed
  # every exec directory already writable; the service runs
  #     find / -type d -writable
  # and asserts it finds EXACTLY 8, so it failed because TOO MUCH was writable.
  # All three reverted approaches were making writable something that already
  # was. Measure before trusting a recorded rationale: this one cost three
  # attempts and several VM runs.
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
  # enough (6.18 >= 5.12) that upstream's version gate lets it run, and
  # testservice-34-check-idmapped.service fails to start. That is id-mapped
  # mount support for exec directories, a different feature from anything
  # above, and it has not been investigated.
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
