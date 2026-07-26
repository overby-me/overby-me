{
  name = "58-REPART";
  testTimeout = 600;
  # De-masked 2026-07-27 after finding the real defect. Two earlier rationales
  # were wrong:
  #   - "systemd-repart stub only": wrong, repart is implemented.
  #   - "definition DISCOVERY is the gap": also wrong. `--definitions=` is
  #     parsed and honoured (parse_args and load_definitions both have unit
  #     tests). The "No partition definitions found." line came from
  #     systemd-repart.service running at boot with no repart.d on the system,
  #     which is benign; it was not the failing command.
  #
  # THE ACTUAL DEFECT, in step 1 of testcase_basic:
  #     systemd-repart --empty=create --size=1G --seed="$seed" "$imgs/zzz"
  # is invoked with NO --definitions at all, and must still write a 1G image
  # holding an empty GPT. rust-systemd returned early twice before doing so:
  # once on an empty definition set, and again on `!has_changes` because no
  # partition was being added. Upstream never stops there: it reads the
  # definitions and carries straight on to find_root, resize_backing_fd and
  # context_load_partition_table. The caller then failed on the missing file:
  #     sfdisk: cannot open /var/tmp/test-repart.imgs.XXXX/zzz: No such file
  #
  # Fixed by treating "the disk had no label and we are writing one" as a
  # change in its own right, and by not refusing an empty definition set under
  # --empty=create. A third divergence surfaced from the same expected output:
  # a fresh GPT must report first-lba 2048, libfdisk's 1 MiB grain, not 34.
  # crates/repart/src/main.rs test_full_empty_create_without_definitions pins
  # all three against the exact sfdisk output this test asserts.
  #
  # Left running unmasked to find the next genuine failure; the test is long
  # and later cases are not expected to pass yet.
  extraUnits = [
    "systemd-repart.service"
  ];
}
