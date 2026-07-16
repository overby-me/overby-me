{
  name = "17-UDEV";
  # `psmisc` provides `killall`, which the test uses to terminate
  # the long-running `RUN+=/usr/bin/sleep 1000` udev worker once
  # ID_PROCESSING=1 has been observed.  Not in the default NixOS
  # shell environment.
  extraPackages = pkgs: [pkgs.psmisc pkgs.procps];
  testEnv = {
    TEST_MATCH_SUBTEST = "\\.device_is_processing\\.sh$";
  };
  # `killall sleep` reliably returns "no process found" on this
  # NixOS test driver because `/proc/<pid>/comm` for a process
  # exec'd via `Command::new("/usr/bin/sleep")` is set to the full
  # path `/usr/bin/sleep` (14 chars, fits in TASK_COMM_LEN=16)
  # rather than the basename `sleep`.  killall only matches the
  # 15-char kernel comm exactly against the passed name, so it
  # misses this process.  Replace the killall invocations with a
  # /proc-walk that matches the BASENAME of comm, finding the
  # process regardless of whether comm is `sleep` or
  # `/usr/bin/sleep`.
  patchScript = ''
    helper='{ for __p in /proc/[0-9]*; do __c=$(cat $__p/comm 2>/dev/null || :); [ "$(basename "$__c")" = sleep ] \&\& kill $(basename $__p) 2>/dev/null || :; done; } || :'
    # Use @ as sed delimiter — the helper contains `|` (from `||`)
    # which would otherwise be treated as the sed pattern delimiter.
    sed -i \
      -e "s@^killall sleep\$@$helper@" \
      -e "s@^    killall -KILL sleep\$@    $helper@" \
      TEST-17-UDEV.device_is_processing.sh
  '';
}
