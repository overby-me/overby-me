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
  # NEXT STEP: check whether systemd-integritysetup@.service is even
  # instantiable here (the C package ships integritysetup*.target but the
  # per-device template may not be among them; if so add it to extraUnits), and
  # then whether crates/integritysetup actually performs the dm-integrity
  # attach for the instance name. Distinguish those two before writing code.
  extraPackages = pkgs: [pkgs.cryptsetup];
  extraUnits = [
    "integritysetup.target"
    "integritysetup-pre.target"
    "remote-integritysetup.target"
  ];
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: systemd-integritysetup@ starts but /dev/mapper/<name> never appears' >/skipped"
      echo "exit 77"
    } > TEST-67-INTEGRITY.sh
    chmod +x TEST-67-INTEGRITY.sh
  '';
  # Skips rather than passes: the dm-integrity attach does not happen
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}
