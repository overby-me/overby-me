# Current Status

**🟢 NixOS boots successfully with rust-systemd as PID 1** — reaches `multi-user.target` with login prompt in ~6 seconds (cloud-hypervisor VM, full networking via networkd + resolved). **8,445 unit tests passing** across 75 crates.

| Phase | Status |
|-------|--------|
| Phase 0 — Foundation | ✅ Complete (sd_notify 15/15 — `NOTIFYACCESS=` runtime override now enforced) |
| Phase 1 — Core System | ✅ Complete (path units, sd_notify full protocol: FDSTORE/FDSTOREREMOVE/FDNAME/ERRNO/BUSERROR/EXIT_STATUS/MONOTONIC_USEC/INVOCATION_ID/WATCHDOG_USEC dynamic, exec command line prefixes `+`/`!`/`!!`/`:` fully supported, **watchdog enforcement** — background thread detects WatchdogSec= timeouts and kills unresponsive services with WatchdogSignal=, Restart=on-watchdog fully functional, **emergency/rescue mode** — kernel command line target override via `systemd.unit=`, `emergency`, `rescue`, `single`, `s`, `S`, `-s`, SysV runlevels `1`–`5`) |
| Phase 2 — Essential System Services | 🔶 In progress (udevd partial — worker thread pool done, `net_setup_link` builtin done, **`hwdb` builtin done** with full trie reader/fnmatch, **inotify-based rules auto-reload** with 3s debounce, **network interface renaming** via netlink RTM_SETLINK (NAME=/ID_NET_NAME + MAC/MTU from .link files), logind/hostnamed/timedated/localed D-Bus done, OnCalendar= full parser done, networkd/resolved D-Bus done) |
| Phase 3 — Network Stack | 🔶 Partial (networkd ✅ D-Bus + `.link` parsing + **`.netdev` virtual device creation** via netlink RTM_NEWLINK (30 device kinds: bridge/bond/vlan/vxlan/macvlan/macvtap/ipvlan/veth/wireguard/gre/sit/erspan/geneve/bareudp/xfrm/vrf/dummy/vcan/tun/tap/etc., with full kind-specific IFLA_INFO_DATA for bridge STP/timers/VLAN-filtering, bond mode/MII/LACP/hash-policy, VLAN ID/protocol, VXLAN VNI/group/port/learning, macvlan/ipvlan modes, veth peer, VRF table, tunnel endpoints/keys, geneve VNI, bareudp ethertype, xfrm interface-id) + **IPv6 Router Advertisement handling** with SLAAC address generation (EUI-64 and **RFC 7217 stable privacy**), **IPv6 address lifetime management** (valid/preferred lifetime tracking, RFC 4862 §5.5.3e refresh rules, deprecation and expiration with RTM_DELADDR removal), ICMPv6 RS/RA socket, prefix info/RDNSS/DNSSL/route info/MTU option parsing, link-local address (EUI-64 or stable privacy), default route installation, `IPv6StableSecretAddress=` and `IPv6LinkLocalAddressGenerationMode=` config options, **routing policy rules** (`[RoutingPolicyRule]` section with From/To/Table/Priority/FirewallMark/FirewallMask/IncomingInterface/OutgoingInterface/SourcePort/DestinationPort/IPProtocol/InvertRule/Family/User/SuppressPrefixLength/Type/TypeOfService — netlink RTM_NEWRULE/RTM_DELRULE with full FRA_* attribute support for IPv4 and IPv6), resolved ✅ D-Bus + DNS cache + `/etc/hosts` reading + **split DNS routing** (per-link routing domains with longest-suffix match), timesyncd ✅ D-Bus, timedated ✅ D-Bus, hostnamed ✅ D-Bus, localed ✅ D-Bus, networkd-wait-online ✅) |
| Phase 4 — Extended Services | 🔶 Partial (machined ✅ D-Bus, portabled ✅ D-Bus, homed ✅ D-Bus, nspawn 🔶 basic + **veth pair creation** via netlink RTM_NEWLINK, oomd, coredump, sysext, dissect, firstboot, creds, cryptsetup ✅, veritysetup ✅, integritysetup ✅, repart ✅) |
| Phase 5 — Utilities, Boot & Polish | 🔶 Partial (analyze, cgls, cgtop, mount, socket-activate, ac-power, detect-virt, generator framework) |

## Unit File Directive Coverage

414 of 429 upstream systemd directives supported (97%). Per-section breakdown:

| Section | Supported | Partial | Unsupported | Total | Coverage |
|---------|-----------|---------|-------------|-------|----------|
| systemd.unit | 87 | 0 | 1 | 88 | 99% |
| systemd.service | 34 | 0 | 0 | 34 | 100% |
| systemd.exec | 145 | 2 | 0 | 147 | 99% |
| systemd.socket | 58 | 0 | 2 | 60 | 97% |
| systemd.resource-control | 46 | 0 | 2 | 48 | 96% |
| sd_notify | 15 | 0 | 0 | 15 | 100% |
| systemd.kill | 7 | 0 | 0 | 7 | 100% |
| systemd.timer | 14 | 0 | 0 | 14 | 100% |
| systemd.path | 7 | 0 | 1 | 8 | 88% |
| systemd.slice | 3 | 0 | 0 | 3 | 100% |
| systemd.device | 1 | 0 | 0 | 1 | 100% |
| systemd.swap | 4 | 0 | 0 | 4 | 100% |

Legend: ✅ = complete, 🔶 = partial, ❌ = not started

Note: `../../UPSTREAM-MAP.md` records verified semantic divergences from upstream
systemd v258 per subsystem; where it conflicts with the coverage claims above, the
map is the audited source of truth.
