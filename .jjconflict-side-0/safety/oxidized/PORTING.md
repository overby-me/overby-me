# The porting method

Every tree under `rust/` is a Rust replacement for a piece of the Linux user space,
and they should all be built the same way: against an upstream oracle, with an honest
ledger of what is not covered. This document is the method, distilled from the largest
port (systemd: 375k lines, 440 upstream integration tests registered, zero fake
passes), and it is binding for the rest.

The two integration trees are the point of the portfolio:

- **[nixos/](nixos/README.md)** (rust-nixos): NixOS with a Rust user space, swapping
  runtime components one at a time.
- **[nixpkgs/](nixpkgs/README.md)** (rust-nixpkgs): Rust replacements for the C
  toolchain that builds Nix packages.

A port is finished when it is switched on in one of those and survives real use. Its
own green numbers are the means, never the finish line.

## The rules

Numbered so reviews and commit messages can cite them.

1. **Oracle first.** Before feature work, wire the upstream test suite (or corpus) so
   it runs against the port and fails honestly. A port with no oracle wired is a
   prototype, whatever its line count.
2. **Honest denominator.** Register the full upstream suite, not a curated subset.
   "N/N passing (100%)" means nothing when the port chose N. Every excluded test is a
   ledger entry with a reason and what removing it would take.
3. **The ledger.** Each port keeps `docs/TEST-OVERRIDES.md` (exemplar:
   [systemd/docs/TEST-OVERRIDES.md](systemd/docs/TEST-OVERRIDES.md)) recording every
   skip, weakening or substitution, classified. No silent skips: a skipped test fails
   CI unless declared. No fake passes, ever.
4. **Arbitrate with the reference.** Provide a `c-<port>-test-*` variant that runs the
   reference implementation through the same harness, and classify every failure as
   environmental or real before debugging it.
5. **Follow upstream's architecture.** Deviations are documented debt with named
   consequences (exemplar:
   [systemd/docs/ARCHITECTURE.md](systemd/docs/ARCHITECTURE.md)). The systemd port's
   one architectural improvisation, its threading model, produced its worst bug class.
6. **Declare purity.** Pure Rust, Rust over bundled C, or hybrid, stated in the
   README. The memory-safety argument covers only the pure parts: `xz` currently
   wraps the C liblzma, so its decompression path inherits C's bug classes and must
   say so until that is resolved.
7. **Audit from outside.** Schedule sampling sweeps that re-run wrappers and verify
   claims from the outside. Assume the green metric gets gamed: in systemd it was,
   roughly 26 times, including by the agents doing the work, and restoring the
   curated assertions surfaced five real bugs.
8. **Pin and chase drift.** The README states the upstream version the oracle came
   from. A scheduled job bumps the pin so upstream drift arrives as failing tests
   rather than silent parity decay.
9. **Ship increments.** "Done" for any milestone means the port is used somewhere
   real: the devshell, the rust-nixpkgs stdenv, or a rust-nixos machine.
10. **Keep a falsification checkpoint.** Each port states what evidence would freeze
    or descope it, and when that decision gets made (exemplar: the strategy section
    of [systemd/docs/ROADMAP.md](systemd/docs/ROADMAP.md)).

## Oracle classes

- **A, upstream suite**: the reference project's own tests run against our binary.
- **B, conformance corpus**: an external standard's corpus with byte-exact expected
  output. The strongest oracle; the video decoders should use ITU conformance
  bitstreams.
- **C, differential**: feed the same input to reference and port and compare outputs.
  Scales directly into fuzzing.
- **D, ecosystem**: the world as the oracle. Build nixpkgs packages with the port in
  the toolchain, or boot and run a machine with the port in the system closure.

Most ports want A (or B) for correctness, C for their untrusted-input surfaces, and
graduate through D.

## Maturity ladder

| Level | Meaning |
|-------|---------|
| L0 | Compiles; no oracle wired |
| L1 | Own tests pass; oracle still unwired |
| L2 | Full upstream suite registered with a ledger; failures honest |
| L3 | Suite green apart from ledgered overrides; differential fuzz on parser surfaces |
| L4 | Shipped increment: used by the devshell, rust-nixpkgs or rust-nixos |
| L5 | Default in this repository's systems |

## Status, measured 2026-08-02

LOC counts `.rs` lines excluding `target/`. "Suite" marks whether a `*.nix` in the
port references an upstream testsuite (a heuristic snapshot; re-derive before relying
on it). "Oracle" is the designated one, not necessarily wired yet. Portfolio total:
862k lines across 36 trees.

| Port | LOC | Designated oracle | Suite | Ledger |
|------|-----|-------------------|-------|--------|
| systemd | 375.1k | A+C+D: upstream integration suite, C oracle, NixOS VM | Y | Y |
| gcc | 218.5k | A: DejaGnu torture; D: stdenv builds | n | n |
| bash | 49.9k | A: upstream `tests/` (verify denominator) | Y | n |
| perl | 43.2k | A: upstream `t/` (denominator currently curated) | Y | n |
| binutils | 22.4k | A: DejaGnu; D: stdenv | Y | n |
| meson | 19.3k | A: upstream test cases; D | Y | n |
| pkg-config | 15.9k | A: upstream checks; C: nixpkgs `.pc` corpus | n | n |
| curl | 15.0k | A: runtests; C: differential | Y | n |
| make | 13.6k | A: GNU suite; D: stdenv | Y | n |
| pipewire | 12.0k | A plus a VM session harness on the systemd pattern | Y | n |
| flatpak | 9.5k | A: installed-tests; D | Y | n |
| tar | 6.6k | A: GNU suite; C: interop corpus | Y | n |
| h265-decoder | 6.2k | B: ITU conformance; C vs ffmpeg; fuzz | n | n |
| awk | 6.1k | A: full gawk suite (currently the BASIC_TESTS subset) | Y | n |
| file | 5.6k | A: magic regression suite; C over a corpus | Y | n |
| h264-decoder | 5.4k | B: ITU conformance; C vs ffmpeg; fuzz | n | n |
| ninja | 4.3k | A: upstream tests incl. `ninja_test` (currently python subset) | Y | n |
| sed | 3.6k | A: GNU suite | Y | n |
| cachix | 3.6k | C: differential vs the upstream CLI | n | n |
| patch | 3.5k | A: GNU suite; fuzz (CVE-2018-1000156 class) | Y | n |
| direnv | 3.5k | A: upstream Go tests ported; C: differential | n | n |
| xz | 2.5k | A: xz-utils suite; purity resolution first (rule 6) | Y | n |
| pcre2 | 2.4k | A: pcre2test; C: differential fuzz vs C PCRE2 | n | n |
| grep | 2.4k | A: GNU suite | Y | n |
| bison | 2.2k | A: upstream testsuite | n | n |
| gzip | 1.7k | A: GNU suite; C: interop | Y | n |
| patchelf | 1.7k | A: upstream tests; C: store-wide differential | Y | n |
| bubblewrap | 1.5k | A: upstream tests | n | n |
| wclip | 1.3k | C: differential under a headless compositor | Y | n |
| texinfo | 0.9k | A: upstream tests | n | n |
| diffutils | 0.9k | A: GNU suite | n | n |
| h26xtoav1 | 0.9k | C: golden outputs vs an ffmpeg pipeline | n | n |
| bzip2 | 0.3k | A: upstream; C: interop | Y | n |
| help2man | 0.3k | C: differential over real binaries | n | n |
| nixos | n/a | Is the class-D oracle (runtime) | Y | n/a |
| nixpkgs | n/a | Is the class-D oracle (build-time) | Y | n/a |

## The denominator audit: the portfolio's main integrity gap

The systemd history says this is where rot starts: its integrity sweep found roughly
26 faked or weakened tests behind healthy-looking numbers. Today every other port
reports a "100%" whose denominator the port itself chose. Known-curated examples:

- awk: 242/242 is gawk's BASIC_TESTS subset, not the full suite.
- perl: "269/269 of the testable-on..." is curation by its own wording.
- ninja: upstream's C++ `ninja_test` cases are not part of the 18/18.
- bash: verify whether the 77 nix integration tests cover upstream's full `tests/`.

The first Tier-0 act for every port: enumerate the full upstream suite, register all
of it, and move every exclusion into a ledger entry (rules 2 and 3). Expect this
alone to find real bugs; in systemd it did.

## Priorities

1. **Denominator audit portfolio-wide.** Cheapest integrity win, and it generates
   each port's honest work list.
2. **One shipped increment per class-D tree.** A rust-nixpkgs stdenv chain (make,
   patchelf, pkg-config, file, tar, gzip, xz, sed, grep, awk, diffutils, patch)
   building a nontrivial package set in CI; and rust-nixos rung 1, one always-on
   service swap (systemd strategy item 1 lands here).
3. **The security tier.** Differential fuzz for parsers of untrusted input: file,
   the h264/h265 decoders (conformance corpus plus fuzz), curl, tar, patch, pcre2,
   and xz once its purity is resolved. These are the ports where memory safety is
   the actual argument.
4. **The daemon tier.** pipewire adopts the systemd VM-harness pattern: a
   `testsuite.nix` analogue driving upstream tests plus a real session.
5. **Shared drift CI** (rule 8), once 1 and 2 exist.

## Starting a new port

Pick targets with (a) a runnable oracle, (b) either privilege and CVE density or
bootstrap value for rust-nixpkgs, and (c) a bounded surface. Then do Tier 0 first:
harness, full-suite registration, ledger, reference-oracle variant. Feature work
begins only when the port can fail honestly.
