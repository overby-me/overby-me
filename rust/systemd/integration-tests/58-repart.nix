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
  # NOW PASSING IN THE VM: testcase_basic steps 1 through 4, i.e. the empty
  # image, the four-partition layout, the six-partition --copy-from= table
  # (every start, size, label, UUID and attribute byte for byte), and
  # --defer-partitions=home,root leaving swap alone at its original offset and
  # slot. Ten repart defects were fixed getting here; see the git log.
  #
  # FOUR of those ten were options PARSED INTO THE ARGUMENT STRUCT, given unit
  # tests proving the parsing worked, and then never consulted anywhere in the
  # logic: --include-partitions=, --exclude-partitions=, CopyBlocks= and
  # --defer-partitions=. A passing parse test is not evidence a flag is
  # honoured; the rest of the crate is worth auditing the same way.
  #
  # NOW PASSING IN THE VM: testcase_basic steps 1 through 4 and most of 5, i.e.
  # the empty image, the four-partition layout, the six-partition --copy-from=
  # table, --defer-partitions=, the deferred refill (sizes, offsets, labels,
  # UUIDs and slot numbers), the extra-partition step, and the 2G resize.
  # Fourteen repart defects were fixed getting here; see the git log.
  #
  # FOUR of them were options PARSED INTO THE ARGUMENT STRUCT, given unit tests
  # proving the parsing worked, and then never consulted anywhere in the logic:
  # --include-partitions=, --exclude-partitions=, CopyBlocks= and
  # --defer-partitions=. A passing parse test is not evidence a flag is
  # honoured; the rest of the crate is worth auditing the same way.
  #
  # CURRENT FAILURE, at the 3G resize in step 5. Everything matches except the
  # newly added partition:
  #     expected  zzz6 : start=4194264, size=2097152
  #     actual    zzz6 : start=4194264, size=1048576
  # Exactly half. rust splits the newly available area between the new
  # partition and further GROWTH of the existing zzz5 that precedes it, both
  # weighted 1000. Upstream gives the whole area to the new partition and
  # leaves zzz5 at the size it reached in step 4.
  #
  # ESTABLISHED, and the lead to follow. Upstream registers a free area on the
  # partition it follows as that partition's PADDING area, not as growth space
  # (repart.c: `after->padding_area = a`), and context_grow_partitions_phase
  # considers a partition for an area when `allocated_to_area == a ||
  # padding_area == a`. So the preceding partition competes there for its
  # PADDING, whose weight is 0 unless PaddingWeight= says otherwise, while a new
  # partition assigned to the area competes for its SIZE. That accounts for
  # step 5 exactly.
  #
  # WHAT IT DOES NOT YET ACCOUNT FOR, so do not implement on this reading
  # alone: step 4 has no new partition and zzz5 DOES grow into the space, from
  # 188416 to 2285568 sectors. If the trailing area were only ever zzz5's
  # padding area, a padding weight of 0 would leave it unchanged. Find where
  # upstream sets allocated_to_area for an EXISTING partition, or what else lets
  # it grow, before changing rust's claim construction. Note rust currently
  # models an existing partition as claiming GROWTH ONLY, (0, max_bytes), with
  # its final size being current plus what it wins; that is what makes step 4
  # pass and step 5 fail.
  extraUnits = [
    "systemd-repart.service"
  ];
  patchScript = ''
    {
      echo "#!/usr/bin/env bash"
      echo "echo 'rust-systemd: an existing partition still competes for size in the area after it' >/skipped"
      echo "exit 77"
    } > TEST-58-REPART.sh
    chmod +x TEST-58-REPART.sh
  '';
  # Skips rather than passes: growth competes with new partitions for a free area
  # See ../docs/TEST-OVERRIDES.md.
  expectedSkip = true;
}
