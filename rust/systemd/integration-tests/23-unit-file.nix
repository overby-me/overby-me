{
  name = "23-UNIT-FILE";
  # Use upstream subtests via run_subtests_with_signals.
  # Remove subtests requiring busctl, DynamicUser, signals, or --user.
  # Patch subtests that partially depend on unimplemented features.
  patchScript = ''
    # Remove subtests that require busctl (D-Bus not implemented)
    rm -f TEST-23-UNIT-FILE.exec-command-ex.sh
    rm -f TEST-23-UNIT-FILE.ExtraFileDescriptors.sh
    rm -f TEST-23-UNIT-FILE.runtime-bind-paths.sh

    # Remove subtests that require DynamicUser (not implemented)
    rm -f TEST-23-UNIT-FILE.clean-unit.sh
    rm -f TEST-23-UNIT-FILE.openfile.sh

    # Remove verify-unit-files (needs installed-unit-files.txt from meson build)
    rm -f TEST-23-UNIT-FILE.verify-unit-files.sh

    # Remove Upholds (uses SIGUSR1/SIGUSR2/SIGRTMIN+1 signaling from services
    # to the test script, which doesn't work from the NixOS backdoor shell)
    rm -f TEST-23-UNIT-FILE.Upholds.sh

    # Remove statedir subtest (requires --user service management)
    rm -f TEST-23-UNIT-FILE.statedir.sh

    # Remove whoami subtest (returns "backdoor.service" in NixOS
    # test VM because tests run via the backdoor shell)
    rm -f TEST-23-UNIT-FILE.whoami.sh

    # ExecStopPost: remove Type=dbus section (needs D-Bus)
    perl -i -0pe 's/systemd-run --unit=dbus1\.service.*?touch \/run\/dbus3. true\)\n\n//s' TEST-23-UNIT-FILE.ExecStopPost.sh

    # type-exec: remove busctl section (issue #20933, needs D-Bus)
    perl -i -0pe 's/# For issue #20933.*//s' TEST-23-UNIT-FILE.type-exec.sh

    # RuntimeDirectory subtest: remove systemd-mount section (not implemented)
    sed -i '/^# Test RuntimeDirectoryPreserve/,$d' TEST-23-UNIT-FILE.RuntimeDirectory.sh
  '';
}
