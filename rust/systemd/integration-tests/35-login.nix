{
  name = "35-LOGIN";
  # Re-baselined 2026-07-27. The previous rationale, "the logind session suite
  # is not implemented", was wrong: crates/logind is 7,237 lines and the suite
  # gets a long way in. setup_test_user, `systemctl edit`, restarting
  # systemd-logind, testcase_ambient_caps and most of testcase_background all
  # pass, sessions are created, PAMName= is honoured, and the background-light
  # class correctly leaves the user manager alone.
  #
  # THE FIRST FAILURE is:
  #     systemd-run -u ... -p Environment=XDG_SESSION_CLASS=background
  #     systemctl is-active user@1002.service   ->  inactive, expected active
  #
  # A first attempt at explaining that said rust-systemd has no `systemd --user`
  # mode. That was WRONG and is retracted: run_user_manager() exists in
  # service_manager.rs and is substantial (XDG unit dirs, its own control
  # socket, subreaper, signal handling, a transient dir for `systemd-run
  # --user`), and logind already names the unit, constructing
  # `user@<uid>.service` for its User objects.
  #
  # Wiring the C package's units in was tried and was NOT enough: class=none and
  # class=background-light correctly left the manager inactive, but class=background
  # did too. The reason is now measured rather than guessed: `grep StartUnit`
  # across all of crates/logind returns NOTHING. logind records
  # `user@<uid>.service` in its User struct and never starts any unit at all.
  #
  # TWO PIECES were missing, and both are now implemented:
  #   1. logind starts user@<uid>.service on session creation, gated on
  #      SESSION_CLASS_WANTS_SERVICE_MANAGER (user, user-early, greeter,
  #      lock-screen, background), which is exactly the distinction the three
  #      systemd-run cases in testcase_background assert. It starts on a detached
  #      thread: pam_systemd calls CreateSession from inside the service PID 1 is
  #      still starting, so blocking for the job would deadlock against PID 1.
  #   2. a rust user@.service. The C package ships one under example/systemd/system
  #      whose ExecStart= is the C binary, so linking it would have run the C
  #      manager under rust PID 1. default.nix now writes rust's own into both
  #      lib/ and example/ (extraUnits searches example/ first), with
  #      Type=notify-reload, which works because run_user_manager() already sends
  #      READY=1 once its control socket is bound.
  #
  # Note run_user_manager() itself was ALREADY substantial before any of this:
  # XDG unit dirs, its own control socket, subreaper, a transient dir. The
  # missing part was never the manager, only the unit and the trigger.
  #
  # THIRD FIX, and the one that actually made the class distinction real. With
  # the manager starting, the failure INVERTED: class=none now reported active,
  # which the test asserts must not happen. The gate was not at fault. logind
  # started the manager exactly twice, both times for a session pam_systemd had
  # announced as class=background:
  #     pam_systemd(...): Asking logind to create session: ... class=background
  # for all three cases, even though the units set
  # Environment=XDG_SESSION_CLASS=none and =background-light. The variable never
  # reached PAM, so every case collapsed onto the one class that DOES want a
  # manager.
  #
  # run_pam_session() called pam_start, pam_acct_mgmt, pam_setcred and
  # pam_open_session, and never pam_putenv, so a unit's Environment= was
  # invisible to its own PAM stack. The process environment is no substitute:
  # config.env is not applied until much later, after the UID drop. Upstream
  # publishes it into the handle first (src/core/exec-invoke.c setup_pam:
  # pam_putenv -> pam_setcred(PAM_ESTABLISH_CRED) -> pam_open_session), and
  # rust now does the same in that order.
  #
  # This is a CENTRAL-PATH change: it affects every PAMName= service, not just
  # this test, so it is validated in a VM before being committed.
  #
  # ALL THREE CLASS ASSERTIONS NOW PASS, VM-confirmed: class=none -> inactive,
  # class=background-light -> inactive, class=background -> active.
  #
  # FOURTH FIX. The test then stopped at
  #     loginctl | grep lightuser | grep -w background-light
  # and the cause was measured rather than assumed. pam_systemd DOES compute the
  # light class correctly for a systemd-sysusers system user:
  #     Asking logind to create session: uid=100 ... class=background-light
  # so nothing was wrong with the class. rust's `loginctl` list-sessions simply
  # printed SESSION/UID/USER/SEAT/TTY and had no CLASS column, which made the
  # session class unobservable from the command line. The struct it deserialises
  # already carried class, leader and since. Columns now match upstream's
  # list_sessions() exactly: session, uid, user, seat, leader, class, tty, idle,
  # since.
  #
  # THE LIGHTUSER ASSERTIONS NOW PASS TOO. What is left is the run0 block, and
  # investigating it found TWO defects rather than the expected one. Neither was
  # visible from the failing line, which is why the passing run0 assertions were
  # checked as well as the failing one.
  #
  # FIFTH FIX, run0 created NO SESSION AT ALL. Upstream run.c adds
  # `PAMName=systemd-run0` to the transient unit (line 1253) so logind registers
  # a session; rust's run0 set no PAM stack, so there was nothing for loginctl to
  # report. testsuite.nix now also enables security.pam.services.systemd-run0,
  # because the C package ships lib/pam.d/systemd-run0 but NixOS never wires it
  # into /etc/pam.d, and pam_start() fails without an /etc/pam.d entry.
  #
  # SIXTH FIX, and the reason three run0 assertions PASSED WITHOUT DESERVING TO:
  # logind never reaped a session whose leader had exited. The leader-alive check
  # in the loader only prunes session FILES inherited from a previous logind; a
  # session created in-process stayed in the table forever. So after the earlier
  # `systemctl stop bggg...`, the dead lightuser sessions were still listed, and
  # `loginctl | grep lightuser | grep -w background-light` matched one of those
  # rather than anything run0 had done. list-sessions now reaps dead leaders
  # through the existing release_session(), which keeps seat and user bookkeeping
  # consistent.
  #
  # Note --lightweight=no asserts with `grep -w background`, which ALSO matches
  # "background-light" because - is a word boundary. That assertion cannot fail
  # on class alone and is not evidence the mapping is right.
  #
  # SEVENTH FIX, the actually-failing line: `run0 -u root` must imply lightweight
  # mode. Ported from run.c: --lightweight=BOOL|auto, defaulting to true when
  # escalating to root (logind cannot tell run0-on-a-TTY from a getty login, so
  # upstream overrides it explicitly), then mapping to XDG_SESSION_CLASS as
  # lightweight ? (pty ? (root ? user-early-light : user-light) : background-light)
  #             : (pty ? (root ? user-early      : user)       : background).
  # An explicit -E XDG_SESSION_CLASS= wins, as upstream. This only works at all
  # because of the pam_putenv fix above: setting the variable on the unit would
  # otherwise never reach pam_systemd.
  # testcase_background NOW PASSES END TO END, VM-confirmed (BEGIN and END both
  # logged), run0 section included. The next testcase, which no earlier run ever
  # reached because set -e aborted first, is testcase_list_users_sessions_seats.
  # It was briefly misread as a regression from the fixes above; it is not, it is
  # newly reached ground.
  #
  # EIGHTH FIX, and the widest-reaching one yet. create_session() drops
  #     ExecStart=-agetty --autologin logind-test-user --noclear %I $TERM
  # into getty@tty2.service.d/ and agetty then failed with
  #     /dev/dumb: cannot open as standard input
  # agetty's syntax is `agetty [options] line [termtype]`, so it should have got
  # `tty2 dumb`; it got `dumb` as the LINE. %I had expanded to nothing, shifting
  # $TERM into the tty slot.
  #
  # %I is implemented (specifier.rs:311). The defect was that drop-in content and
  # base content were specifier-resolved with a HARDCODED EMPTY INSTANCE:
  # resolve_specifiers(content, unit_name, ""). So %i and %I in a drop-in on any
  # instantiated template silently expanded to nothing, not just here. Four call
  # sites in units/loading/ now derive the instance from the unit name via a new
  # instance_of() (getty@tty2.service -> tty2; plain and template units -> "").
  #
  # NINTH FIX: loginctl's last column was the session start time; upstream's is
  # the IDLE timestamp, blank while the session is not idle (loginctl.c passes
  # idle_timestamp_monotonic). testcase_list_users_sessions_seats asserts $9 is
  # '-', which the old column could never satisfy.
  #
  # SEPARATE BUG FOUND HERE, and SINCE FIXED, not this test's blocker:
  # units/loading/directory_deps.rs resolve_specifiers() used to call
  # system_specifier_context() unconditionally, so SpecifierContext::for_user()
  # was dead code and every unit a USER manager loaded got system specifier
  # values. %t resolved to /run rather than $XDG_RUNTIME_DIR, which is why the
  # user dbus.socket tried to bind /run/bus; %S, %C and %h were wrong the same
  # way. There is now a manager_specifier_context() (directory_deps.rs) that
  # picks for_user() when SYSTEMD_USER_MANAGER is set, and every call site
  # funnels through the one resolve_specifiers() wrapper that uses it;
  # system_specifier_context() has no callers left.
  #
  # TENTH FIX, elsewhere but worth recording because it was the same class of
  # user-manager defect: the generated user@.service carried no Delegate=, so
  # the manager ran as the user while its cgroup stayed root-owned and every
  # service it started died with "Couldnt create service cgroup: Permission
  # denied". Fixed by declaring Delegate=pids memory cpu as upstream does.
  #
  # These notes are a running history, so read them as "what was true when
  # written" and check the code before acting on any of them.
  # WHERE IT STOPS NOW, and the C ORACLE CANNOT ARBITRATE.
  # testcase_list_users_sessions_seats fails in check_session with "no session
  # or multiple sessions". The %I fix above did work: agetty no longer dies on
  # /dev/dumb. What happens instead is that the autologin session opens and
  # closes at once:
  #     login[N]: pam_unix(login:session): session closed for user logind-test-user
  #     logind: Released session 33
  #     getty@tty2.service -> ServiceExited
  # so agetty and login both run and the user's shell exits immediately.
  # /usr/bin/bash IS symlinked by testsuite.nix, and TTYPath=/StandardInput=tty
  # handling does exist in exec_helper.rs, so neither of those is the cause.
  # Left undiagnosed rather than guessed at.
  #
  # c-systemd-test-35-login was run to decide environmental-vs-defect and CANNOT
  # settle it. The C oracle fails EARLIER, in testcase_ambient_caps, where a
  # `systemd-run -p PAMName= -p Type=oneshot -p User=logind-test-user` unit will
  # not start at all. The reason is a PACKAGING ASYMMETRY, not a behavioural gap:
  # default.nix's mkPamWithSystemd symlinks pam_systemd.so into linux-pam's
  # securedir and feeds that libpam to the rust build via the libsystemd PAM_LIB
  # crate override, because rust's exec_helper dlopens libpam and the 35-LOGIN
  # PAM stack references the module by bare name. The C build gets stock NixOS
  # pam, whose securedir omits pam_systemd.so since NixOS pam.d files normally
  # use absolute module paths. So the two managers do not even load the same
  # libpam here, and the oracle cannot be cited as evidence either way for this
  # test.
  #
  # rust-systemd does still get further through the file than the C build does
  # in this VM, but that is a statement about the packaging, not a claim that
  # rust is more correct than upstream.
  #
  # NO OVERRIDE: the red result is honest, and there are ~13 further testcases
  # after this one that have never been reached.
  extraUnits = [
    "user@.service"
    "user-runtime-dir@.service"
    "user.slice"
  ];
  testTimeout = 600;
}
