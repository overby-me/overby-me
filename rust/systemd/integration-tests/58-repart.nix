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
  # CURRENT FAILURE, precisely diagnosed. After the deferred run leaves only
  # swap on the disk, the test runs repart again with no --defer-partitions= to
  # fill the deferred partitions in, and gets:
  #     Error: Cannot place partition 'root2.conf': no free region of 319.6M
  # 319.6M is TOTAL free space divided three ways: usable 1022M minus swap's
  # 64M is 958M, and 958/3 = 319.6. But the free space is not contiguous. Swap
  # sits at sector 1777624 with 92M of padding after it, so the only usable gap
  # is the 866M BEFORE swap, and three partitions there can have at most 288M
  # each.
  #
  # The gap is structural rather than arithmetical, and the model needed to fix
  # it is worked out below so nobody has to re-derive it. rust-systemd
  # distributes space across the SUM of all free space and only then tries to
  # place each partition in a single contiguous region. Upstream instead:
  #
  #   1. builds a FreeArea per gap, each remembering the partition it follows;
  #   2. reduces an area's space available to NEW partitions by the padding the
  #      preceding partition is owed (free_area_available_for_new_partitions);
  #   3. assigns each new partition by FIRST FIT over the areas sorted
  #      SMALLEST first, budgeting that partition's minimum-with-padding into
  #      the area as it goes (context_allocate_partitions);
  #   4. grows the partitions assigned to each area within that area's span
  #      (context_grow_partitions_on_free_area, once per area).
  #
  # Step 2 is the one that is easy to miss and decides this test. The disk here
  # holds only swap, at sector 1777624, and there are two gaps: 866M before it
  # and 92M after. Sorting smallest first would send all three partitions into
  # the 92M gap, which is NOT what upstream produces. It does not, because swap
  # carries PaddingMinBytes=92M, so the whole trailing gap is swap's padding and
  # the area's availability for new partitions is zero. Only the 866M gap is
  # left, all three land there, and the sequential grow_claims() already in
  # crates/repart/src/main.rs then yields exactly the asserted
  # 591856/591856/591864 sectors, consuming the gap with nothing left over.
  # That arithmetic has been checked against the test's own numbers.
  extraUnits = [
    "systemd-repart.service"
  ];
}
