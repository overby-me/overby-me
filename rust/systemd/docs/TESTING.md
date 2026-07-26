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
