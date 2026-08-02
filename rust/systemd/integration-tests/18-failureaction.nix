{
  name = "18-FAILUREACTION";
  # The upstream script is two phases across a reboot, keyed on /firstphase:
  #
  #   systemd-run --wait -p FailureAction=poweroff true
  #   (! systemd-run --wait -p SuccessAction=poweroff false)
  #   if ! test -f /firstphase ; then
  #       echo OK >/firstphase
  #       systemd-run --wait -p SuccessAction=reboot true      <- phase 1
  #   else
  #       echo OK >/testok
  #       systemd-run --wait -p FailureAction=exit \
  #           -p FailureActionExitStatus=123 false             <- phase 2
  #   fi
  #   sleep infinity
  #
  # NOTE the first two lines assert the NON-triggering direction: `true` under
  # FailureAction= and `false` under SuccessAction= mean neither action ever
  # fires. They pass whether or not the actions work at all, so the real
  # coverage is entirely in the two phases.
  #
  # PHASE 1 IS RESTORED as of 2026-07-30. The previous note claimed "the NixOS
  # test VM cannot survive SuccessAction=reboot (QEMU hard reset)", which was
  # stale: the harness grew reboot-resume machinery (testsuite.nix:848-872,
  # whose comment names this very test) that reconnects and re-runs the script,
  # and `allowReboot` below starts QEMU with allow_reboot=True. The script
  # tracks its own progress across boots via /firstphase, which is exactly the
  # shape that machinery expects. `sleep infinity` moves into the then-branch to
  # park the script until the reboot lands: without it the script would exit 0
  # before the machine goes down and the harness would check /testok too early.
  #
  # PHASE 2 STAYS MASKED, and the reason is the harness, not rust. Upstream does
  # NOT exit PID 1 here: emergency-action.c:153-170 degrades "exit" to
  # "poweroff" for a system manager, because exiting the machine's init panics
  # the kernel. Either way the machine ends, and our harness checks /testok from
  # the HOST after the script returns, so there would be no machine left to ask.
  # /testok is already written by then, so deleting the line loses the
  # FailureActionExitStatus=123 assertion and nothing else.
  #
  # That assertion was worth having: the exit status was silently ignored. It is
  # now implemented (parsed value plumbed through to the exit) and covered by
  # unit tests in activate.rs instead, since this harness cannot host it.
  allowReboot = true;
  patchScript = ''
    # Ends the machine, and the /testok check runs on the host afterwards.
    sed -i '/FailureAction=exit/d' TEST-18-FAILUREACTION.sh
    # Park inside phase 1 instead, so the script stays alive until the reboot
    # lands. Bounded rather than upstream's `sleep infinity`: if the reboot does
    # NOT happen, the script falls out of the branch, writes no /testok and
    # fails in five minutes instead of sitting out the 1800s testTimeout.
    sed -i '/^sleep infinity$/d' TEST-18-FAILUREACTION.sh
    sed -i '/SuccessAction=reboot/a\    sleep 300' TEST-18-FAILUREACTION.sh
  '';
}
