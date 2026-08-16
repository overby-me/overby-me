# Testing

Three layers: workspace unit tests, differential tests against real systemd, and the
upstream systemd integration suite in a NixOS VM.

## Unit tests

```sh
cargo test --workspace          # or: just test
cargo test -p libsystemd
cargo test -p libsystemd -- journal
```

About 9,700 test functions across 93 crates.

### Robustness fuzzers

Several parsers that consume attacker-influenced or corrupt input carry an in-tree,
dependency-free fuzzer: a seeded LCG assembles random inputs and each is run through the
parser under `std::panic::catch_unwind`. Where a parse can loop (calendar/time
`next_elapse` searches), a worker thread is joined under a 30s wall-clock budget so a
hang, not just a panic, fails the test. They take no new dependencies and run in a few
seconds each. Grep for `fn fuzz_`:

- binary journal objects (`c_journal`, `entry`) and the export format,
- the unit-file parser (`parse_file` + `parse_service`, including size/rlimit values),
- the calendar parser (`CalendarSpec::parse` + chained `next_elapse`),
- the `systemd-analyze` time parsers (`parse_timestamp`, `TimeSpan::parse`),
- the resolved/networkd wire parsers,
- the `systemd-network-generator` kernel-command-line parser + generator
  (`parse_cmdline` + `generate` over random `ip=`/`vlan=`/`bond=`/`rd.route=`/… soup),
- the `systemd-fstab-generator` fstab parser + emitters (`parse_fstab` + `emit_*`).

The time fuzzer caught a real overflow panic on an out-of-range year, and the journal
fuzzers caught amplification and underflow DoS bugs; the rest were clean nets. When adding
a parser over untrusted bytes, add a matching fuzzer.

## Differential tests

`crates/difftest` runs identical input through rust-systemd and through real systemd and
compares outputs, with built-in normalizers for timestamps, PIDs, boot IDs, machine IDs,
addresses and non-deterministic ordering.

```sh
just difftest                     # all
just difftest-list                # enumerate
just difftest-report              # JUnit + JSON + Markdown into result/
just difftest-update-snapshots    # approve current outputs as golden
```

### In-process differential oracles

A lighter-weight, host-independent complement runs a corpus through both the rust
binary and the corresponding C binary in the same process and asserts they agree. Each
oracle is a `#[test]` gated on an env var naming the C binary, so plain `cargo test` and
CI without the C tools skip it silently. `just differential` resolves the C binaries
from `nixpkgs#systemd` and sets every gate:

```sh
just differential
```

Covered so far:

- journal export-format parsing (`systemd-journal-remote`);
- unit-name escaping (`systemd-escape`: escape / path / mangle / template / suffix /
  unescape, plus the option-validation and path-warning error paths);
- `systemd-analyze` `timespan` / `calendar` / `timestamp` / `exit-status` /
  `condition` / `compare-versions` / `capability` / `architectures`;
- the `systemd-id128` `show` table, the `-a APP` app-specific derivation, and the
  pretty / JSON output formats;
- the `systemd-creds list` empty-set `ENXIO` exit contract;
- `systemd-network-generator`: `ip=`, `ifname=`, `net.ifname_policy=`, `net.ifnames=`,
  `rd.route=`, and the merged `vlan=`/`bond=`/`bridge=` per-interface `.netdev`/`.network`
  files (whole generated file trees, byte-for-byte);
- `systemd-fstab-generator`: device-node canonicalization, `blockdev@` ordering,
  `SourcePath`/`Documentation`/`Where`/`Type`, and the `.requires`/`.wants` symlink tree.

Together these have found and fixed roughly two dozen real drift bugs, from a unitless
`timespan` read as microseconds instead of seconds and a UTC-suffixed `timestamp`
silently zeroing its seconds, to `systemd-network-generator` dropping an `rd.route=`
that shared an interface with `ip=`, and `systemd-fstab-generator` leaving `UUID=`
device specs unresolved and never ordering mounts after their `blockdev@` target.

Each oracle compares the *semantic* result, not raw stdout, and deliberately does not
flag these intentional differences:

- **Presentation.** Labels and column widths differ; `timespan` compares the μs value,
  `calendar`/`timestamp` the `Normalized form:` line, `exit-status` the name/class
  columns. rust's `timestamp` also prints an extra `(in UTC):` line, and the wall-clock
  `From now:` line is not compared.
- **Lenient vs strict arguments.** `systemd-analyze exit-status` warns and continues on a
  non-numeric argument where C errors and aborts, so the corpus feeds only numeric
  statuses.
- **No timezone database.** `systemd-analyze timestamp` evaluates in UTC and rejects a
  non-UTC zone rather than misparsing it; the corpus uses UTC-anchored inputs.
- **Environment-gated wiring.** `systemd-fstab-generator`'s fsck dependencies (both the
  per-mount `systemd-fsck@` and the root `systemd-fsck-root.service`) are gated in C on
  `sysfs_check()`, so their presence depends on the host and is excluded; the oracle
  compares a per-unit set of the environment-independent fields plus the non-fsck symlink
  tree, so header field order and section layout are also ignored.
- **Parsed-but-dropped inputs.** `systemd-network-generator` accepts `team=` and
  `net.ifnames=` but, like C, writes no file for them (`team=` is not even a C
  kernel-command-line option, and `net.ifnames=` is udev's concern). C also parses
  `bond=` options and then drops them, so the `.netdev` carries no `[Bond]` section.
  Everything the generator *does* emit — `ip=`/`ifname=`/`net.ifname_policy=`/`rd.route=`
  and the merged `vlan=`/`bond=`/`bridge=` `70-<ifname>.netdev`/`.network` files — is
  byte-faithful.

## Integration tests

Boots a NixOS VM with rust-systemd as PID 1, installs the upstream systemd test scripts
and testdata, runs one test, and checks for the `/testok` marker.

```sh
nix build .#checks.x86_64-linux.rust-systemd-test-01-basic -L
nix build .#checks.x86_64-linux.rust-systemd-test-74-aux-utils-cat -L
```

Never run plain `nix flake check`: this repo has thousands of check derivations and a
single evaluator peaks over 30 GiB. Use `just check` from the repo root, which drives
memory-capped `nix-eval-jobs` workers. A full run takes about an hour.

### The C systemd oracle

Every test is registered twice. `c-systemd-test-<name>` runs the same wrapper against
upstream C systemd instead of rust-systemd:

```sh
nix build .#checks.x86_64-linux.c-systemd-test-54-creds -L
```

This is the way to settle whether a failure is a rust-systemd defect or an artefact of
the NixOS VM. If the C build fails the same way with the override removed, the problem
is environmental and belongs in the permanent-exclusion list rather than the fix list.

### How a test is registered

`default.nix` reads every `*.nix` in `integration-tests/` (`lib.readDir`, no central
list) and turns each into a check named after the file. Adding a file is enough.

Each file is an attribute set consumed by `testsuite.nix`:

| Key | Purpose |
|-----|---------|
| `name` | Upstream family, e.g. `"07-PID1"`. Selects `test/units/TEST-<name>.sh`. |
| `testEnv.TEST_MATCH_SUBTEST` | Regex selecting one subtest of a multi-subtest family. |
| `patchScript` | Shell run in the unpacked test directory before execution. |
| `extraPackages` | Extra packages on the VM's PATH. |
| `testTimeout` | Seconds, default 1800. |
| `enableTpm` | Attach a software TPM at `/dev/tpmrm0`. Needed by TEST-70-TPM2. |
| `allowReboot` | Guest `systemctl reboot` restarts the VM instead of terminating QEMU. |
| `useBootLoader` | Boot through a real bootloader from a disk image, so a reboot re-runs the full boot and the driver's backdoor is re-established. Implies `allowReboot`. |

**A new `.nix` file is invisible to Nix until git tracks it.** In this colocated jj repo,
`jj commit` alone has not been sufficient. Run `git add <file>` and confirm with
`git ls-files` before spending a VM run. Edits to already-tracked files are picked up
without this.

### Why testsuite.nix patches stage 2

`testsuite.nix:143` overrides `system.build.bootStage2` with a copy of NixOS's
`stage-2-init.sh` that has this block stripped:

```bash
if test -w /dev/kmsg; then
    exec > >(tee -i /proc/self/fd/"$logOutFd" | while read -r line; do
        echo "<7>stage-2-init: $line" > /dev/kmsg
    done) 2>&1
fi
```

The process-substitution fd setup races against parallel kernel module auto-load during
early boot. If the subshell does not hook up fd 1 before the parent's next write, init
stalls with no panic and never execs `systemd`. This caused roughly a 30% VM-test flake
rate. NixOS on C systemd never hits it because `boot.initrd.systemd.enable` short-circuits
past the block, and rust-systemd does not ship a stage-1 systemd initrd. Everything else
in stage 2 is preserved verbatim; the only loss is the `<7>stage-2-init:` kmsg re-log.

Do not remove this override without re-measuring the flake rate.

### patchScript policy

`patchScript` exists for facts about the NixOS VM that differ from upstream's test
image, not for hiding failures. Legitimate uses:

- Absolute paths for bare commands in inline unit files (NixOS has no `/bin`, and
  inline units are subject to a compiled-in `DEFAULT_PATH_NORMAL`).
- Deleting upstream's `systemctl --no-block exit 123` line, which is its own VM-teardown
  convention; the NixOS driver uses the `/testok` marker instead.
- Appending `touch /testok` to a subtest that upstream runs under a parent harness.
- Genuine upstream typos, with the upstream line cited.

Anything that deletes an assertion, replaces a subtest, or short-circuits to `exit 77`
is an override and must be recorded in [TEST-OVERRIDES.md](TEST-OVERRIDES.md) with what
it would take to remove it.

### Known trap: a skip currently scores as a pass

`testsuite.nix` asserts `test -f /testok -o -f /skipped`, and exit code 77 creates
`/skipped` automatically. A test that runs zero lines therefore reports success. Until
that is changed, a green check is not by itself evidence that a test ran. Check
[TEST-OVERRIDES.md](TEST-OVERRIDES.md).

## Boot testing with rust-nixos

`../nixos` builds a minimal NixOS image with rust-systemd as PID 1 and boots it under
cloud-hypervisor with the serial console captured. From `rust-nixos/`:

```sh
just run                       # interactive, serial output on your terminal
just test                      # automated, streams output, pass/fail
just test-log /tmp/boot.log    # save the full boot log
just test-keep                 # keep the VM up afterwards for poking at it
```

## Debugging early boot failures

When a service crashes during early boot, stderr and the journal are usually gone
already: the mount namespace has hidden `/dev/console` and the journal socket. The exec
helper writes to `/dev/kmsg` through `libsystemd::kmsg_log`, which survives mount
namespace changes and shows up on the serial console.

Enable tracing for one unit:

```ini
[Service]
Environment=SYSTEMD_LOG_LEVEL=trace
```

Level resolution in the exec helper, highest priority first: the `SYSTEMD_LOG_LEVEL`
environment variable, then the level the manager passed in `ExecHelperConfig`, then the
built-in default of `warn`. Numeric syslog levels 0 to 7 are accepted.

Then filter the log:

```sh
grep 'rust-systemd\[systemd-timesyncd\]' /tmp/boot.log
```

After `ProtectKernelLogs=` hides `/dev/kmsg` the logger degrades silently and only
stderr remains. That is intended: the messages that matter for diagnosing sandbox setup
are the ones emitted before that point.
