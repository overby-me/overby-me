# Differential Test Corpus

Golden file corpus for differential testing between rust-systemd and real systemd (v256). Each test category feeds identical inputs through both implementations and asserts equivalent outputs, state transitions, and side effects.

## Directory Structure

```text
corpus/
├── units/          Unit files (.service, .socket, .target, .timer, .path, .mount, .slice, .scope)
├── configs/        Configuration files (system.conf, journald.conf, resolved.conf, etc.)
├── journal/        Journal test data (native protocol messages, syslog inputs, binary fields)
├── network/        Network configuration (.network, .link, .netdev, kernel cmdline fragments)
├── generators/     Generator inputs (/etc/fstab, /etc/crypttab, kernel cmdline for generators)
└── README.md       This file
```

## Sourcing

Files in this corpus are sourced from:

1. **Upstream systemd test suite** — Edge-case unit files from systemd's `test/` directory (v256).
2. **Distribution unit files** — Real-world units shipped by Fedora, Debian, Arch, and NixOS.
3. **Fuzzer-generated inputs** — Edge cases discovered by `cargo-fuzz` differential fuzzers.
4. **Manually crafted regressions** — Files targeting specific parser or runtime behaviors where rust-systemd and real systemd are known to have diverged or where the specification is ambiguous.

## Version Pinning

All corpus files are pinned against **systemd v256** for reproducibility. When testing against other systemd versions (v254, v255), some files may be version-gated via metadata in the test harness.

## Categories

### `units/`

Unit files exercising the full range of INI parser behavior and directive coverage:

- **Quoting** — Single quotes, double quotes, no quotes, escape sequences (`\n`, `\t`, `\\`, `\"`, `\'`, `\x`, `\u`, `\U`).
- **Line continuation** — Backslash at end of line, backslash at EOF, continuation across blank lines.
- **Comments** — `#` and `;` comment styles, inline comments, comments in continuation lines.
- **Specifiers** — All 40+ `%`-specifiers in various contexts, template units with special instance names.
- **Drop-in overlays** — `.d/*.conf` files testing lexicographic ordering, cross-directory priority (`/etc/` > `/run/` > `/usr/lib/`), empty-value reset semantics, multi-value append directives.
- **Template instantiation** — `foo@instance.service` with empty, hyphen, slash, `@`, multi-byte UTF-8, and extremely long instance strings.
- **Directives** — Coverage matrix files for each of the 429+ upstream directives.
- **Boolean parsing** — `yes`/`no`, `true`/`false`, `1`/`0`, `on`/`off`, case-insensitive variants.
- **Duration parsing** — Compound expressions (`1h 30min 5s`), all unit suffixes, `infinity`, zero.
- **Byte size parsing** — SI and IEC suffixes (`K`, `M`, `G`, `T`, `P`, `E`), plain integers.
- **Resource control** — `MemoryMax=`, `CPUQuota=`, `TasksMax=`, `IOWeight=`, `Slice=`, `Delegate=`.
- **Security hardening** — All `Protect*=`, `Restrict*=`, `SystemCallFilter=`, `CapabilityBoundingSet=` variants.
- **Credential directives** — `LoadCredential=`, `SetCredential=`, `ImportCredential=`, encrypted variants.
- **Conditions and asserts** — All 28 condition types with both true and false evaluation inputs.

### `configs/`

Daemon configuration files with drop-in override directories:

- `system.conf` / `user.conf` with all supported directives.
- `journald.conf` — Storage, rate limiting, size limits, forwarding options.
- `resolved.conf` — DNS, DNSSEC, split-DNS routing domains, stub listener.
- `logind.conf` — VT management, idle actions, power key handling.
- `timesyncd.conf` — NTP server lists, poll intervals.
- `oomd.conf` — Memory pressure thresholds.
- `coredump.conf` — Storage and size limits.

### `journal/`

Test inputs for journald write and query compatibility:

- Native protocol messages (structured journal entries with all field types).
- Syslog socket input (RFC 3164 and RFC 5424 formats).
- stdout/stderr capture payloads.
- Binary field encoding edge cases.
- Multi-line messages, NUL bytes, oversized fields.
- Rate-limit burst sequences.

### `network/`

networkd, resolved, and network-generator test configurations:

- `.network` files — DHCP, static addressing, routing policy rules, DNS configuration.
- `.link` files — Interface matching, naming policy, MTU, MAC address overrides.
- `.netdev` files — Bridge, bond, VLAN, VXLAN, veth, dummy, VRF, WireGuard.
- Kernel command line fragments for `systemd-network-generator` (`ip=`, `rd.route=`, `nameserver=`, `vlan=`, `bond=`, `bridge=`, `ifname=`).

### `generators/`

Input files for systemd generators:

- `/etc/fstab` entries for `systemd-fstab-generator`.
- `/etc/crypttab` entries for `systemd-cryptsetup-generator`.
- Kernel command line fragments for `systemd-debug-generator`, `systemd-run-generator`, `systemd-getty-generator`.
- GPT partition table descriptors for `systemd-gpt-auto-generator`.

## Adding New Corpus Files

1. Place the file in the appropriate subdirectory.
2. Name it descriptively (e.g., `quoting-double-with-escapes.service`, `fstab-bind-mount.fstab`).
3. Add a `#[difftest]` test function in the corresponding test module that loads and exercises the file.
4. Run `just difftest` to verify it produces the expected result against both implementations.
5. If the test reveals a known intentional divergence, add an entry to `tests/difftest/known-divergences.toml`.