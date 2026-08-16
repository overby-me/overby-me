# Fe-C v0 detection corpus

Source: SafeFFI's evaluation, Table 1 (arXiv:2510.20688, USENIX Security '26),
merging the ERASan and RustSan datasets. Captured 2026-07-21. This file is the
acceptance corpus behind PLAN.md §7 — the `corpus-rustsec` flake check.

Legend (per tool, our encoding of the paper's marks):
**D** detected · **E** detected *earlier*, at the cast/boundary site ·
**P** partial / different error · **—** missed · Int ✓ = the paper marks it
reachable via allocator/`mem*` interception (informs cementite's interceptor
tiers).

| ID | Int | HWASan | SafeFFI | Fe-C v0 gate |
| -- | --- | ------ | ------- | ------------ |
| CVE-2017-1000430 | ✓ | D | D | required |
| CVE-2018-20991 | ✗ | D | D | required |
| CVE-2018-21000 | ✓ | D | D | required |
| CVE-2019-15551 | ✓ | D | E | required |
| CVE-2019-16140 | ✓ | D | D | required |
| CVE-2019-16882 | ✓ | D | D | required |
| CVE-2019-25009 | ✓ | D | D | required |
| CVE-2020-25574 | ✓ | D | E | required |
| CVE-2020-25791 | ✓ | D | D | required¹ |
| CVE-2020-25792 | ✓ | D | D | required¹ |
| CVE-2020-25795 | ✓ | D | E | required |
| CVE-2020-35858 | ✗ | D | D | required |
| CVE-2020-35860 | ✓ | D | D | required |
| CVE-2020-35861 | ✓ | D | D | required |
| CVE-2020-35891 | ✗ | D | E | required |
| CVE-2020-35892 | ✓ | D | E | required |
| CVE-2020-35893 | ✓ | D | E | required |
| CVE-2020-35906 | ✓ | D | D | required |
| CVE-2020-36434 | ✗ | D | D | required |
| CVE-2020-36464 | ✗ | D | E | required |
| CVE-2020-36465 | ✓ | D | D | required |
| CVE-2021-25900 | ✓ | D | D | required |
| CVE-2021-26954 | ✓ | D | E | required |
| CVE-2021-28028 | ✓ | D | E | required |
| CVE-2021-28031 | ✓ | D | D | required |
| CVE-2021-29933 | ✓ | D | E | required |
| CVE-2021-30455 | ✓ | D | E | required |
| CVE-2021-30457 | ✗ | D | E | required |
| CVE-2021-45694 | ✗ | D | E | required |
| CVE-2021-45713 | ✗ | D | E | required |
| CVE-2021-45720 | ✗ | D | E | required |
| RUSTSEC-2020-0061 | ✗ | — | P | stretch² |
| RUSTSEC-2020-0091 | ✗ | D | E | required |
| RUSTSEC-2020-0097 | ✗ | D | E | required |
| RUSTSEC-2020-0167 | ✓ | D | D | required |
| RUSTSEC-2021-0003 | ✓ | D | D | required |
| RUSTSEC-2021-0031 | ✗ | D | E | required |
| RUSTSEC-2021-0033 | ✓ | D | E | required |
| RUSTSEC-2021-0039 | ✓ | D | E | required |
| RUSTSEC-2021-0047 | ✓ | — | — | known-hard³ |
| RUSTSEC-2021-0048 | ✓ | D | D | required |
| RUSTSEC-2021-0049 | ✓ | D | D | required |
| RUSTSEC-2021-0053 | ✓ | D | D | required |
| RUSTSEC-2022-0070 | ✓ | D | D | required |
| RUSTSEC-2022-0078 | ✓ | D | D | required |
| RUSTSEC-2023-0005 | ✓ | D | E | required |

Notes:

1. ASan (redzone-based) missed 25791/25792 while HWASan caught them —
   cases that favor exact metadata over probabilistic placement. Cementite's
   deterministic table should treat these as must-catch, not lucky-catch.
2. **Stretch**: RUSTSEC-2020-0061 was caught (partially) only by SafeFFI's
   cast-site check — evidence the boundary placement sees things per-access
   sanitizers don't. Fe-C `case` should aim to catch it cleanly.
3. **Known-hard**: RUSTSEC-2021-0047 evaded every evaluated tool. Keep it in
   the corpus as a standing falsifier; catching it is a research result, not a
   gate.
4. 21 of the 46 are marked **E**: SafeFFI reported them at the raw→safe cast
   rather than the eventual bad access. Matching that early-report behavior is
   a v0 UX requirement (`fe_c_ensure` failing at the cast names the unsafe
   block at fault).
5. The Int column justifies interceptor tier (a)/(b) priority in cementite:
   a majority of the corpus is reachable through allocator/`mem*`
   interposition even before MIR instrumentation exists — useful for
   bootstrapping the check before the driver is done.

## Scoreboard (2026-07-25): read this before quoting a catch count

**Against the 46 rows above: 1 confirmed, 1 probable, 44 unresolved.** The
table's IDs were never resolved to `crate@version`, so most rows have not been
attempted, missed, or scored. They are simply unknown.

What actually got built is a second, **opportunistic corpus**, chosen by what
the checker could reach. It is good work and it found real bugs, but it is not
the corpus this file declares, and the two must not be conflated:

| Caught | Crate | In the 46 rows? |
| ------ | ----- | --------------- |
| RUSTSEC-2021-0003 | `smallvec 1.6.0`, heap OOB | **yes** (confirmed) |
| RUSTSEC-2019-0009 | `smallvec 0.6.9`, `grow` UAF | **probably**: likely the alias of CVE-2019-15551, verify against the advisory DB |
| RUSTSEC-2021-0128 | `rusqlite 0.25.3` + bundled SQLite, stack borrow across FFI | no |
| RUSTSEC-2021-0130 | `lru 0.6.6`, heap UAF | no |
| RUSTSEC-2020-0039 | `simple-slab 0.3.2`, OOB read of a `libc::malloc` buffer | no (alias unverified) |
| RUSTSEC-2021-0028 | `toodee 0.2.0`, OOB write | no (alias unverified) |
| RUSTSEC-2022-0079 | `elf_rs 0.2.0`, `from_raw_parts` length lie | no |
| RUSTSEC-2023-0016 | `partial_sort 0.1.1`, debug-assert-only bound | no |
| RUSTSEC-2025-0109 | `binary_vec_io 0.1.12`, `from_raw_parts` length lie | no (postdates the paper) |

**Attempted or surveyed and not caught**, which is the more informative half:

| Not caught | Why | Would need |
| ---------- | --- | ---------- |
| `caja` / `ptr::as_ref` class (RUSTSEC-2026-0130) | The bad access happens in a non-inlined `core` call, outside instrumented MIR | Interprocedural provenance, or `-Zbuild-std` (D1, currently blocked) |
| `bumpalo` class | Custom arena allocator; its memory is never registered | Arena registration, an open design question |
| `atom` class | Concurrency bug; no thread-interleaving story exists | Out of scope for v0 |
| RUSTSEC-2021-0047 | Evaded every tool in the paper | Standing falsifier, not a gate (note 3) |

The honest summary of the harvest: the reachable vein is mined out. New CVE
classes now need an alignment check or interprocedural provenance, not more
reproducers of shapes already covered. See `docs/evaluation-2026-07.md` §3.5.

## To do at the computer

1. **Rescore this file** (Phase E6). For each of the 46 rows: resolve ID →
   `crate@vulnerable-version` using the advisory DB's CVE aliases, then mark it
   caught / missed / not attempted. Publish the denominator. A row that is
   *missed* is worth as much as a row that is caught, and more than a row that
   is silently dropped.
2. For rows worth pursuing: pin the version, add a minimal reproducer, vendor
   sources through `platform/nix/lib/cargo` so the check stays pure and offline, and
   assert the Fe-C report (site + FailKind), not just a nonzero exit.
3. Give each entry a real patched-version control. Today only `smallvec-0003`
   has one (`CLAUDE.md` §5).
