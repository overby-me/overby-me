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
  # THE FREE-AREA SPAN RULE, now implemented, recorded because it took two
  # wrong readings to find. The span a free area offers is NOT just the gap:
  #     span = gap + round_up(preceding partition's extent, grain)
  # (repart.c context_grow_partitions_on_free_area). Folding the preceding
  # partition's own extent in is what lets an EXISTING partition state its
  # claim as a TOTAL size rather than as growth. A partition already larger
  # than its weighted share of that combined span trips the overcharge phase,
  # settles at its current size, and hands the rest of the gap on.
  #
  # That single rule accounts for both steps, which no earlier reading did:
  #   - step 4, a 2G disk with only the existing partition competing: it grows
  #     from 188416 to 2285568 sectors, filling the gap.
  #   - step 5, a 3G disk with a new partition as well: the existing one is
  #     already bigger than half the combined span, so it stays at 2285568 and
  #     the newcomer takes the whole 2097152-sector gap.
  # Sizing against the bare gap gave the newcomer half, and modelling the
  # existing partition as claiming growth only made step 4 pass while step 5
  # still failed. Both are checked in crates/repart unit tests.
  extraUnits = [
    "systemd-repart.service"
  ];
}
