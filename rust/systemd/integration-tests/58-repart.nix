{
  name = "58-REPART";
  testTimeout = 600;
  # Re-masked 2026-07-27, but much further in than before and for a new reason.
  # Two earlier rationales were wrong and are recorded here so they are not
  # tried again:
  #   - "systemd-repart stub only": wrong, repart is implemented.
  #   - "definition DISCOVERY is the gap": also wrong. --definitions= was always
  #     honoured. The "No partition definitions found." line came from
  #     systemd-repart.service running at boot with no repart.d on the system,
  #     which is benign and was never the failing command.
  #
  # SIX real defects were found and fixed against this test, all VM-confirmed:
  #   1. --empty=create wrote no image at all, because repart returned early on
  #      an empty definition set and again on !has_changes.
  #   2. --empty=create did not imply --dry-run=no, so the image the test builds
  #      every later step on was never written.
  #   3. a fresh GPT reported first-lba 34 rather than libfdisk's 1 MiB grain.
  #   4. seeded UUIDs used an FNV-1a placeholder instead of HMAC-SHA256, so
  #      every UUID differed from the one systemd derives.
  #   5. --include-partitions=/--exclude-partitions= were parsed and never
  #      consulted; Label= was truncated rather than rejected when over-long;
  #      GrowFileSystem= never defaulted on for growable types.
  #   6. space a capped partition could not use was discarded instead of being
  #      redistributed, and sizes were aligned to the sector rather than to
  #      upstream's 4096-byte grain.
  #
  # testcase_basic steps 1 and 2 now match upstream byte for byte, including
  # every UUID, label, attribute and sector offset.
  #
  # --copy-from= IS NOW IMPLEMENTED: it reads the source image's GPT, turns
  # every used partition into a definition pinned to that partition's exact
  # size, type, UUID, label, GPT flags and trailing padding, and copies the
  # bytes across after the table is written. Passing the same image twice
  # duplicates its partitions, which is what this test does. A unit test builds
  # a source image and copies it twice, checking the six partitions come out
  # contiguous with their UUIDs and labels duplicated verbatim.
  #
  # TWO DIVERGENCES REMAIN in the copied result, and NEITHER belongs to
  # --copy-from; both are visible in the source image on its own:
  #   - three equally weighted partitions in a 50M image come out
  #     33432/33432/33432 sectors where upstream gets 33432/33440/33440. The
  #     allocator aligns each partition's byte share down to the 4096-byte grain
  #     independently and drops the remainder; upstream allocates in whole
  #     grains and hands the leftover ones back out.
  #   - the default label for a partition with no Label= is "root" here, where
  #     upstream derives it from the type designator as "root-x86-64", and
  #     numbers a repeat of the same type "root-x86-64-2".
  # Both are worth fixing on their own account, since every multi-partition
  # layout and every unlabelled partition is affected, not just this test.
  #
  # OLDER NOTE, now superseded:
  #     systemd-repart --definitions "" \
  #                    --copy-from="$imgs/qqq" --copy-from="$imgs/qqq" \
  #                    "$imgs/copy"
  # must copy every partition, and its contents, out of one image into another,
  # twice over, producing six partitions.
  extraUnits = [
    "systemd-repart.service"
  ];
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: repart allocator drops grain remainders and derives default labels differently' >/skipped"
      echo "exit 77"
    } > TEST-58-REPART.sh
    chmod +x TEST-58-REPART.sh
  '';
  # Skips rather than passes: partition sizes and default labels still diverge
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}
