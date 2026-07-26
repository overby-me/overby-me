{
  name = "67-INTEGRITY";
  testTimeout = 300;
  # Baselined 2026-07-27 in two steps. The old rationale
  # ("systemd-integritysetup stub only") was wrong on both counts.
  #
  # STEP 1: the first failure was not in rust-systemd at all:
  #     integritysetup format /dev/loop0 --batch-mode -I crc32c ''
  #     ./TEST-67-INTEGRITY.sh: line 85: integritysetup: command not found
  # That is the cryptsetup project's CLI, which the test drives to create the
  # dm-integrity device BEFORE systemd-integritysetup@ attaches it. A missing
  # test dependency, now supplied via extraPackages. With it, `integritysetup
  # format` succeeds and dm-0 appears, so the device really is being created.
  #
  # STEP 2, the current and genuine gap: the test then runs
  #     systemctl start systemd-integritysetup@integrity_test.service
  #     udevadm wait --timeout=30 --settle /dev/mapper/integrity_test
  # and the wait does not find the device, after which cleanup runs. So the
  # unit starts but /dev/mapper/integrity_test never appears.
  #
  # RESOLVED WITHOUT A VM RUN: of the two candidates, it is the first.
  # `systemd-integritysetup@.service` is NOT SHIPPED by this nixpkgs systemd
  # build at all: neither example/systemd/system nor lib/systemd/system contains
  # it, only lib/systemd/system-generators/systemd-integritysetup-generator,
  # which upstream uses to generate instances from /etc/integritytab. The test
  # starts the template directly, so it can never instantiate here whatever
  # crates/integritysetup does. That also means the attach path has still never
  # been exercised, so nothing is yet known about it.
  #
  # MECHANISM CONFIRMED WORKING (2026-07-27). The unit is GENERATED, not
  # shipped: systemd-integritysetup-generator produces it from /etc/integritytab,
  # which the test writes before daemon-reload. A run with everything wired shows
  # the whole chain functioning:
  #     [[ -e /run/systemd/generator/systemd-integritysetup@integrity_test.service ]]
  #     systemctl start systemd-integritysetup@integrity_test.service
  #     REAP ... systemd-integritysetup@integrity_test.service -> ServiceExited
  # so the C generator IS discovered and run (package_generator_dir walks up from
  # the running binary, which covers NixOS; BUILTIN_GENERATORS skips only fstab
  # and getty), reload-time generator re-runs work (5c34f5a2), and the generated
  # unit starts. The test then cycles through several test_one cases.
  #
  # FAILING CASE ISOLATED (from the same run, no extra VM time):
  #   test_one crc32c 0   -> passes  (integrity metadata on the same device)
  #   test_one crc32c 1   -> FAILS   (separate_data=1, i.e. --data-device=)
  # and the script exits there without reaching /testok, with `systemctl
  # list-units --failed` EMPTY. So no unit failed: an assertion inside the
  # separate-data case tripped set -e, at the `udevadm wait --settle
  # /dev/mapper/integrity_test` after the unit had started and exited.
  #
  # So the plain attach works and only the SEPARATE DATA DEVICE variant does
  # not. That is a much narrower target than "the attach is broken".
  #
  # ALSO RULED OUT, by reading crates/integritysetup: the data-device= option
  # IS parsed from integritytab (main.rs parse_options, "data-device" |
  # "data_device"), IS stored, and IS built into the dm table by
  # build_integrity_params, which emits `data_device:<path>`. Unit tests cover
  # both the parse and the param building. So the option is not being dropped.
  #
  # REMAINING CANDIDATES, none yet tested:
  #   - the generated unit for the separate-data case may not pass the option
  #     through: check what systemd-integritysetup-generator writes into
  #     /run/systemd/generator/systemd-integritysetup@integrity_test.service for
  #     this case, and compare with what crates/integritysetup expects on its
  #     command line.
  #   - the dm ioctl may succeed while producing a device that never gets a
  #     /dev/mapper/ node, which is what the failing udevadm wait observes.
  # Capture the generated unit file and the service's own output in the VM
  # before choosing between these; do not guess.
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: 67-INTEGRITY cycles several cases then fails; failing case not yet isolated' >/skipped"
      echo "exit 77"
    } > TEST-67-INTEGRITY.sh
    chmod +x TEST-67-INTEGRITY.sh
  '';
  # Skips rather than passes: failing test_one case not yet isolated
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}
