//! BPF-based device access control for cgroup v2.
//!
//! On cgroup v2 there is no `devices` controller.  Instead, device access
//! is restricted by attaching a `BPF_PROG_TYPE_CGROUP_DEVICE` eBPF program
//! to the cgroup.  The program receives a `bpf_cgroup_dev_ctx` context for
//! every device-access attempt and returns 1 (allow) or 0 (deny).
//!
//! This module generates the minimal eBPF bytecode needed to enforce
//! `DevicePolicy=` and `DeviceAllow=` directives, loads it via the `bpf()`
//! syscall, and attaches it to the service's cgroup directory.
//!
//! The `BPF_*` constants and instruction/register helpers below mirror the
//! kernel's eBPF ABI in full so the bytecode builder reads like the kernel
//! headers; the minimal device program only emits a subset, so allow the
//! unused ABI surface rather than trimming it to whatever today's program
//! happens to reference.
#![allow(dead_code)]

use crate::units::unit_parsing::DevicePolicy;
use log::trace;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

// ── BPF syscall constants ────────────────────────────────────────────────
const BPF_PROG_LOAD: libc::c_int = 5;
const BPF_PROG_ATTACH: libc::c_int = 8;
const BPF_PROG_TYPE_CGROUP_DEVICE: u32 = 15;
const BPF_CGROUP_DEVICE: u32 = 6;

// ── Device type / access constants (from linux/bpf.h) ────────────────────
const BPF_DEVCG_DEV_BLOCK: u32 = 1;
const BPF_DEVCG_DEV_CHAR: u32 = 2;
const BPF_DEVCG_ACC_MKNOD: u32 = 1;
const BPF_DEVCG_ACC_READ: u32 = 2;
const BPF_DEVCG_ACC_WRITE: u32 = 4;
const BPF_DEVCG_ACC_ALL: u32 = BPF_DEVCG_ACC_MKNOD | BPF_DEVCG_ACC_READ | BPF_DEVCG_ACC_WRITE;

// ── eBPF instruction encoding ────────────────────────────────────────────
//
// Each eBPF instruction is 8 bytes:
//   u8  code
//   u8  dst_reg:4 | src_reg:4
//   i16 off
//   i32 imm

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BpfInsn {
    code: u8,
    regs: u8, // lower nibble = dst, upper nibble = src
    off: i16,
    imm: i32,
}

// Register numbers
const R0: u8 = 0;
const R1: u8 = 1;
const R2: u8 = 2;
const R3: u8 = 3;
const R4: u8 = 4;
const R5: u8 = 5;
const R6: u8 = 6;
const R7: u8 = 7;
const R8: u8 = 8;
const R9: u8 = 9;

// Instruction helpers
const fn insn(code: u8, dst: u8, src: u8, off: i16, imm: i32) -> BpfInsn {
    BpfInsn {
        code,
        regs: (src << 4) | dst,
        off,
        imm,
    }
}

/// r[dst] = *(u32 *)(r[src] + off)
const fn ldx_mem_w(dst: u8, src: u8, off: i16) -> BpfInsn {
    insn(0x61, dst, src, off, 0) // BPF_LDX | BPF_MEM | BPF_W
}

/// r[dst] = r[src]  (64-bit move)
const fn mov64_reg(dst: u8, src: u8) -> BpfInsn {
    insn(0xbf, dst, src, 0, 0) // BPF_ALU64 | BPF_MOV | BPF_X
}

/// r[dst] = imm  (64-bit)
const fn mov64_imm(dst: u8, imm: i32) -> BpfInsn {
    insn(0xb7, dst, 0, 0, imm) // BPF_ALU64 | BPF_MOV | BPF_K
}

/// r[dst] &= imm  (32-bit)
const fn alu32_and_imm(dst: u8, imm: i32) -> BpfInsn {
    insn(0x54, dst, 0, 0, imm) // BPF_ALU | BPF_AND | BPF_K
}

/// r[dst] >>= imm  (64-bit)
const fn alu64_rsh_imm(dst: u8, imm: i32) -> BpfInsn {
    insn(0x77, dst, 0, 0, imm) // BPF_ALU64 | BPF_RSH | BPF_K
}

/// if r[dst] != imm goto pc + 1 + off
const fn jne_imm(dst: u8, imm: i32, off: i16) -> BpfInsn {
    insn(0x55, dst, 0, off, imm) // BPF_JMP | BPF_JNE | BPF_K
}

/// exit (return r0)
const fn exit_insn() -> BpfInsn {
    insn(0x95, 0, 0, 0, 0) // BPF_JMP | BPF_EXIT
}

/// r[dst] = *(u8 *)(r[src] + off)
const fn ldx_mem_b(dst: u8, src: u8, off: i16) -> BpfInsn {
    insn(0x71, dst, src, off, 0) // BPF_LDX | BPF_MEM | BPF_B
}

/// r[dst] += imm  (64-bit)
const fn alu64_add_imm(dst: u8, imm: i32) -> BpfInsn {
    insn(0x07, dst, 0, 0, imm) // BPF_ALU64 | BPF_ADD | BPF_K
}

/// if r[dst] > r[src] (unsigned) goto pc + 1 + off
const fn jgt_reg(dst: u8, src: u8, off: i16) -> BpfInsn {
    insn(0x2d, dst, src, off, 0) // BPF_JMP | BPF_JGT | BPF_X
}

/// if (u32)r[dst] == imm goto pc + 1 + off  (32-bit compare, no sign-extension)
const fn jeq32_imm(dst: u8, imm: i32, off: i16) -> BpfInsn {
    insn(0x16, dst, 0, off, imm) // BPF_JMP32 | BPF_JEQ | BPF_K
}

/// if (u32)r[dst] != imm goto pc + 1 + off  (32-bit compare)
const fn jne32_imm(dst: u8, imm: i32, off: i16) -> BpfInsn {
    insn(0x56, dst, 0, off, imm) // BPF_JMP32 | BPF_JNE | BPF_K
}

// ── Device rule representation ───────────────────────────────────────────

struct DeviceRule {
    dev_type: Option<u32>, // None = any type
    major: Option<u32>,    // None = any major
    minor: Option<u32>,    // None = any minor
    access: u32,           // bitmask of BPF_DEVCG_ACC_*
}

// ── Default pseudo-devices for "closed" policy ───────────────────────────

fn default_closed_rules() -> Vec<DeviceRule> {
    vec![
        // /dev/null    char 1:3
        DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: Some(1),
            minor: Some(3),
            access: BPF_DEVCG_ACC_ALL,
        },
        // /dev/zero    char 1:5
        DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: Some(1),
            minor: Some(5),
            access: BPF_DEVCG_ACC_ALL,
        },
        // /dev/full    char 1:7
        DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: Some(1),
            minor: Some(7),
            access: BPF_DEVCG_ACC_ALL,
        },
        // /dev/random  char 1:8
        DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: Some(1),
            minor: Some(8),
            access: BPF_DEVCG_ACC_ALL,
        },
        // /dev/urandom char 1:9
        DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: Some(1),
            minor: Some(9),
            access: BPF_DEVCG_ACC_ALL,
        },
        // /dev/tty     char 5:0
        DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: Some(5),
            minor: Some(0),
            access: BPF_DEVCG_ACC_ALL,
        },
        // /dev/console char 5:1
        DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: Some(5),
            minor: Some(1),
            access: BPF_DEVCG_ACC_ALL,
        },
        // /dev/ptmx    char 5:2
        DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: Some(5),
            minor: Some(2),
            access: BPF_DEVCG_ACC_ALL,
        },
        // /dev/pts/*   char 136:*
        DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: Some(136),
            minor: None,
            access: BPF_DEVCG_ACC_ALL,
        },
    ]
}

// ── Parse DeviceAllow= entries ───────────────────────────────────────────

fn parse_access(s: &str) -> u32 {
    let mut access = 0u32;
    for c in s.chars() {
        match c {
            'r' => access |= BPF_DEVCG_ACC_READ,
            'w' => access |= BPF_DEVCG_ACC_WRITE,
            'm' => access |= BPF_DEVCG_ACC_MKNOD,
            _ => {}
        }
    }
    access
}

fn parse_device_allow_entry(entry: &str) -> Option<DeviceRule> {
    let parts: Vec<&str> = entry.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let device = parts[0];
    let access = if parts.len() > 1 {
        parse_access(parts[1])
    } else {
        BPF_DEVCG_ACC_ALL
    };

    if access == 0 {
        return None;
    }

    // Wildcard device classes
    if device == "char-*" {
        return Some(DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_CHAR),
            major: None,
            minor: None,
            access,
        });
    }
    if device == "block-*" {
        return Some(DeviceRule {
            dev_type: Some(BPF_DEVCG_DEV_BLOCK),
            major: None,
            minor: None,
            access,
        });
    }

    // Specific device path — stat to get major:minor
    if device.starts_with('/') {
        match std::fs::metadata(device) {
            Ok(meta) => {
                let rdev = meta.rdev();
                let major = libc::major(rdev) as u32;
                let minor = libc::minor(rdev) as u32;
                let mode = meta.mode();
                let dev_type = if mode & libc::S_IFBLK != 0 {
                    BPF_DEVCG_DEV_BLOCK
                } else {
                    BPF_DEVCG_DEV_CHAR
                };
                return Some(DeviceRule {
                    dev_type: Some(dev_type),
                    major: Some(major),
                    minor: Some(minor),
                    access,
                });
            }
            Err(e) => {
                trace!("Cannot stat device {device} for DeviceAllow: {e}");
                return None;
            }
        }
    }

    None
}

// ── BPF program generation ───────────────────────────────────────────────
//
// For each allow-rule we emit a block of instructions that checks the
// device type, major, minor and access mask against the rule.  If all
// checks pass the program returns 1 (ALLOW).  Checks for wildcard
// fields (None) are omitted.  After all rules, we emit the default
// return value (0 = DENY for strict/closed, 1 = ALLOW for auto).
//
// Context layout (bpf_cgroup_dev_ctx):
//   offset 0: access_type  (bits 0-15 = dev type, bits 16-31 = access)
//   offset 4: major
//   offset 8: minor

fn build_bpf_program(rules: &[DeviceRule], default_allow: bool) -> Vec<BpfInsn> {
    let mut insns: Vec<BpfInsn> = Vec::new();

    for rule in rules {
        let block_start = insns.len();
        let mut skip_patches: Vec<usize> = Vec::new();

        // Load access_type into r2, save copy in r3
        insns.push(ldx_mem_w(R2, R1, 0)); // r2 = ctx->access_type
        insns.push(mov64_reg(R3, R2)); // r3 = r2
        insns.push(alu32_and_imm(R2, 0xFFFF)); // r2 = device type

        // Check device type (skip if doesn't match)
        if let Some(dev_type) = rule.dev_type {
            skip_patches.push(insns.len());
            insns.push(jne_imm(R2, dev_type as i32, 0)); // placeholder off
        }

        // Check major
        if let Some(major) = rule.major {
            insns.push(ldx_mem_w(R2, R1, 4)); // r2 = ctx->major
            skip_patches.push(insns.len());
            insns.push(jne_imm(R2, major as i32, 0)); // placeholder off
        }

        // Check minor
        if let Some(minor) = rule.minor {
            insns.push(ldx_mem_w(R2, R1, 8)); // r2 = ctx->minor
            skip_patches.push(insns.len());
            insns.push(jne_imm(R2, minor as i32, 0)); // placeholder off
        }

        // Check access: (requested_access & ~allowed_access) must be 0
        let disallowed = (!rule.access) & BPF_DEVCG_ACC_ALL;
        if disallowed != 0 {
            insns.push(alu64_rsh_imm(R3, 16)); // r3 = requested access
            insns.push(alu32_and_imm(R3, disallowed as i32)); // r3 &= disallowed mask
            skip_patches.push(insns.len());
            insns.push(jne_imm(R3, 0, 0)); // if any disallowed bit → skip
        }

        // Match: allow
        insns.push(mov64_imm(R0, 1));
        insns.push(exit_insn());

        // Patch all skip-jumps in this block to land here (start of next rule)
        let skip_target = insns.len();
        for &patch_idx in &skip_patches {
            insns[patch_idx].off = (skip_target - patch_idx - 1) as i16;
        }

        let _ = block_start; // suppress unused warning
    }

    // Default return
    insns.push(mov64_imm(R0, if default_allow { 1 } else { 0 }));
    insns.push(exit_insn());

    insns
}

// ── BPF syscall wrappers ─────────────────────────────────────────────────

fn bpf_prog_load(insns: &[BpfInsn], prog_type: u32) -> Result<i32, String> {
    let license = b"GPL\0";
    // Use a zeroed buffer large enough for the bpf_attr union.
    let mut attr = [0u8; 128];

    // prog_type at offset 0
    attr[0..4].copy_from_slice(&prog_type.to_ne_bytes());
    // insn_cnt at offset 4
    attr[4..8].copy_from_slice(&(insns.len() as u32).to_ne_bytes());
    // insns pointer at offset 8
    attr[8..16].copy_from_slice(&(insns.as_ptr() as u64).to_ne_bytes());
    // license pointer at offset 16
    attr[16..24].copy_from_slice(&(license.as_ptr() as u64).to_ne_bytes());

    let ret = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            BPF_PROG_LOAD as libc::c_long,
            attr.as_ptr() as libc::c_long,
            attr.len() as libc::c_long,
        )
    };

    if ret < 0 {
        return Err(format!(
            "BPF_PROG_LOAD failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(ret as i32)
}

fn bpf_prog_attach(prog_fd: i32, cgroup_path: &Path, attach_type: u32) -> Result<(), String> {
    // Open the cgroup directory with O_DIRECTORY to ensure correct fd type
    let path_cstr = std::ffi::CString::new(
        cgroup_path
            .to_str()
            .ok_or_else(|| format!("Non-UTF8 cgroup path: {cgroup_path:?}"))?,
    )
    .map_err(|e| format!("Invalid cgroup path: {e}"))?;

    let target_fd = unsafe { libc::open(path_cstr.as_ptr(), libc::O_DIRECTORY | libc::O_RDONLY) };
    if target_fd < 0 {
        return Err(format!(
            "Cannot open cgroup dir {cgroup_path:?}: {}",
            std::io::Error::last_os_error()
        ));
    }

    // Buffer must be large enough for the kernel's expected bpf_attr size.
    // Use a generous zero-initialized buffer to accommodate any kernel version.
    let mut attr = [0u8; 128];
    attr[0..4].copy_from_slice(&(target_fd as u32).to_ne_bytes());
    attr[4..8].copy_from_slice(&(prog_fd as u32).to_ne_bytes());
    attr[8..12].copy_from_slice(&attach_type.to_ne_bytes());
    attr[12..16].copy_from_slice(&0u32.to_ne_bytes());

    let ret = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            BPF_PROG_ATTACH as libc::c_long,
            attr.as_ptr() as libc::c_long,
            attr.len() as libc::c_long,
        )
    };

    unsafe {
        libc::close(target_fd);
    }

    if ret < 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!(
            "BPF_PROG_ATTACH failed for {cgroup_path:?} (target_fd={target_fd}, prog_fd={prog_fd}): {err}"
        ));
    }
    Ok(())
}

// ── Public API ───────────────────────────────────────────────────────────

/// Apply `DevicePolicy=` and `DeviceAllow=` by loading and attaching an
/// eBPF device-filter program to the service's cgroup.
///
/// Does nothing for `DevicePolicy=auto` with no explicit `DeviceAllow=`
/// entries (the default — no restrictions).
pub fn apply_device_policy(
    cgroup_path: &Path,
    device_policy: &DevicePolicy,
    device_allow: &[String],
) -> Result<(), String> {
    if *device_policy == DevicePolicy::Auto && device_allow.is_empty() {
        return Ok(());
    }

    let mut rules = Vec::new();

    // For "closed" policy, add default pseudo-device allowances
    if *device_policy == DevicePolicy::Closed {
        rules.extend(default_closed_rules());
    }

    // Add explicit DeviceAllow= entries
    for entry in device_allow {
        if let Some(rule) = parse_device_allow_entry(entry) {
            rules.push(rule);
        }
    }

    let default_allow = *device_policy == DevicePolicy::Auto;
    let program = build_bpf_program(&rules, default_allow);

    trace!(
        "Loading BPF device program ({} insns, {} rules, default={}) for cgroup {cgroup_path:?}",
        program.len(),
        rules.len(),
        if default_allow { "allow" } else { "deny" },
    );

    let prog_fd = bpf_prog_load(&program, BPF_PROG_TYPE_CGROUP_DEVICE)?;
    let result = bpf_prog_attach(prog_fd, cgroup_path, BPF_CGROUP_DEVICE);

    // Close the program fd — the kernel keeps its own reference after attach.
    unsafe {
        libc::close(prog_fd);
    }

    result
}

// ── RestrictNetworkInterfaces= (BPF_PROG_TYPE_CGROUP_SKB) ─────────────────

const BPF_PROG_TYPE_CGROUP_SKB: u32 = 8;
const BPF_CGROUP_INET_INGRESS: u32 = 0;
const BPF_CGROUP_INET_EGRESS: u32 = 1;
/// Offset of `ifindex` in `struct __sk_buff` (uapi/linux/bpf.h).
const SK_BUFF_IFINDEX_OFF: i16 = 40;

/// Build a cgroup/skb program that inspects the packet's interface index
/// (`__sk_buff.ifindex`). For an allow-list, packets on a listed interface are
/// accepted (return 1) and all others dropped (return 0); a deny-list inverts
/// this. Straight-line, so the verifier accepts it trivially.
fn build_restrict_ifaces_program(ifindices: &[u32], deny_list: bool) -> Vec<BpfInsn> {
    let (match_ret, default_ret) = if deny_list { (0, 1) } else { (1, 0) };
    let mut insns = vec![ldx_mem_w(R2, R1, SK_BUFF_IFINDEX_OFF)]; // r2 = ctx->ifindex
    for &idx in ifindices {
        // if r2 != idx, skip the next two insns; else return match_ret.
        insns.push(jne_imm(R2, idx as i32, 2));
        insns.push(mov64_imm(R0, match_ret));
        insns.push(exit_insn());
    }
    insns.push(mov64_imm(R0, default_ret));
    insns.push(exit_insn());
    insns
}

/// Apply `RestrictNetworkInterfaces=` by attaching a cgroup/skb filter to the
/// service's cgroup (both ingress and egress). An allow-list permits only the
/// listed interfaces; a deny-list (`~eth0`) blocks the listed ones. Interface
/// names are resolved to indices via if_nametoindex(); names not present are
/// skipped. A blocked packet's send fails with EPERM (kernel behaviour for a
/// dropped cgroup/skb egress), never a kill.
pub fn apply_restrict_network_interfaces(
    cgroup_path: &Path,
    interfaces: &[String],
) -> Result<(), String> {
    if interfaces.is_empty() {
        return Ok(());
    }
    let deny_list = interfaces[0].starts_with('~');
    let mut ifindices: Vec<u32> = Vec::new();
    for (i, name) in interfaces.iter().enumerate() {
        let n = if i == 0 {
            name.strip_prefix('~').unwrap_or(name)
        } else {
            name.as_str()
        };
        let n = n.trim();
        if n.is_empty() {
            continue;
        }
        let cname = match std::ffi::CString::new(n) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let idx = unsafe { libc::if_nametoindex(cname.as_ptr()) };
        if idx == 0 {
            trace!("RestrictNetworkInterfaces: interface '{n}' not present, ignoring");
            continue;
        }
        if !ifindices.contains(&idx) {
            ifindices.push(idx);
        }
    }
    if ifindices.is_empty() {
        // Nothing resolvable: for a deny-list there is nothing to block, and for
        // an allow-list installing a drop-everything filter would break the
        // service. Either way, do not install one.
        return Ok(());
    }

    let program = build_restrict_ifaces_program(&ifindices, deny_list);
    let prog_fd = bpf_prog_load(&program, BPF_PROG_TYPE_CGROUP_SKB)?;
    let ingress = bpf_prog_attach(prog_fd, cgroup_path, BPF_CGROUP_INET_INGRESS);
    let egress = bpf_prog_attach(prog_fd, cgroup_path, BPF_CGROUP_INET_EGRESS);
    unsafe {
        libc::close(prog_fd);
    }
    ingress.and(egress)
}

// ── IPAddressAllow= / IPAddressDeny= (BPF_PROG_TYPE_CGROUP_SKB) ───────────
//
// The packet's address (source on ingress, destination on egress) is masked
// against each configured prefix; the longest-matching prefix wins (allow beats
// deny on a tie). Both IPv4 and IPv6 are filtered: the program reads the IP
// version nibble and takes the matching path. A packet whose version is neither
// 4 nor 6, or which is too short to parse, is allowed.

/// Offsets of `data` / `data_end` in `struct __sk_buff`.
const SK_BUFF_DATA_OFF: i16 = 76;
const SK_BUFF_DATA_END_OFF: i16 = 80;
/// IPv4 header field offsets (relative to the start of the L3 header, which is
/// where a cgroup/skb program's packet data begins).
const IPV4_SRC_OFF: i16 = 12;
const IPV4_DST_OFF: i16 = 16;
/// IPv6 header field offsets (16-byte source at 8, destination at 24).
const IPV6_SRC_OFF: i16 = 8;
const IPV6_DST_OFF: i16 = 24;
/// `BPF_JMP | BPF_JA`: an unconditional jump by `off`.
const BPF_JA: u8 = 0x05;

/// Parse an `A.B.C.D[/len]` string into `(network, mask, prefixlen)` where the
/// network and mask are in the byte order a BPF `LDX_W` of the packet address
/// produces (little-endian load of the wire bytes), so a direct masked compare
/// against the loaded address works.
fn parse_ipv4_cidr(s: &str) -> Option<(u32, u32, u8)> {
    let (addr_s, len) = match s.split_once('/') {
        Some((a, l)) => (a, l.trim().parse::<u8>().ok()?),
        None => (s, 32),
    };
    if len > 32 {
        return None;
    }
    let addr: std::net::Ipv4Addr = addr_s.trim().parse().ok()?;
    let mask_be: u32 = if len == 0 { 0 } else { (!0u32) << (32 - len) };
    let mask_octets = mask_be.to_be_bytes();
    let a = addr.octets();
    let net_octets = [
        a[0] & mask_octets[0],
        a[1] & mask_octets[1],
        a[2] & mask_octets[2],
        a[3] & mask_octets[3],
    ];
    Some((
        u32::from_le_bytes(net_octets),
        u32::from_le_bytes(mask_octets),
        len,
    ))
}

/// Resolve one `IPAddress{Allow,Deny}=` entry (a CIDR or a systemd special name)
/// to its IPv4 prefix. IPv6 entries return None (deferred).
fn ipv4_prefix_from_entry(entry: &str) -> Option<(u32, u32, u8)> {
    match entry.trim() {
        "any" => Some((0, 0, 0)),
        "localhost" => parse_ipv4_cidr("127.0.0.0/8"),
        "link-local" => parse_ipv4_cidr("169.254.0.0/16"),
        "multicast" => parse_ipv4_cidr("224.0.0.0/4"),
        e if e.contains(':') => None, // IPv6, handled separately
        e => parse_ipv4_cidr(e),
    }
}

/// Parse an IPv6 `addr[/len]` string into `(net_words, mask_words, prefixlen)`,
/// each of the four words in the byte order a BPF `LDX_W` of that 4-byte group
/// produces (little-endian load of the wire bytes), so a per-word masked compare
/// against the loaded address works.
fn parse_ipv6_cidr(s: &str) -> Option<([u32; 4], [u32; 4], u8)> {
    let (addr_s, len) = match s.split_once('/') {
        Some((a, l)) => (a, l.trim().parse::<u8>().ok()?),
        None => (s, 128),
    };
    if len > 128 {
        return None;
    }
    let addr: std::net::Ipv6Addr = addr_s.trim().parse().ok()?;
    let a = addr.octets();
    // The 16-byte prefix mask: the high `len` bits set (network bit order).
    let mut mask_octets = [0u8; 16];
    let mut bits = len as usize;
    for byte in mask_octets.iter_mut() {
        if bits >= 8 {
            *byte = 0xff;
            bits -= 8;
        } else if bits > 0 {
            *byte = 0xffu8 << (8 - bits);
            bits = 0;
        } else {
            break;
        }
    }
    let mut net_words = [0u32; 4];
    let mut mask_words = [0u32; 4];
    for i in 0..4 {
        let m = [
            mask_octets[i * 4],
            mask_octets[i * 4 + 1],
            mask_octets[i * 4 + 2],
            mask_octets[i * 4 + 3],
        ];
        let n = [
            a[i * 4] & m[0],
            a[i * 4 + 1] & m[1],
            a[i * 4 + 2] & m[2],
            a[i * 4 + 3] & m[3],
        ];
        net_words[i] = u32::from_le_bytes(n);
        mask_words[i] = u32::from_le_bytes(m);
    }
    Some((net_words, mask_words, len))
}

/// Resolve one `IPAddress{Allow,Deny}=` entry to its IPv6 prefix, if any. The
/// special names expand to their IPv6 halves (matching systemd); a bare IPv4
/// entry returns None.
fn ipv6_prefix_from_entry(entry: &str) -> Option<([u32; 4], [u32; 4], u8)> {
    match entry.trim() {
        "any" => Some(([0; 4], [0; 4], 0)),
        "localhost" => parse_ipv6_cidr("::1/128"),
        "link-local" => parse_ipv6_cidr("fe80::/64"),
        "multicast" => parse_ipv6_cidr("ff00::/8"),
        e if e.contains(':') => parse_ipv6_cidr(e),
        _ => None, // bare IPv4 or a plain name: not an IPv6 prefix
    }
}

/// Build a cgroup/skb program that reads the IP version and checks the packet
/// address (at the IPv4/IPv6 source or destination offset, per `egress`) against
/// the sorted `v4` and `v6` prefix lists (each `(network, mask, verdict)`,
/// longest-prefix first). The first matching prefix's verdict is returned;
/// unmatched packets of a filtered family get `default_verdict`; packets that are
/// neither IPv4 nor IPv6, or too short to parse, are allowed. Forward jumps to the
/// shared ALLOW/DEFAULT tails and the IPv6 path are patched after layout.
fn build_ip_filter_program(
    v4: &[(u32, u32, i32)],
    v6: &[([u32; 4], [u32; 4], i32)],
    default_verdict: i32,
    egress: bool,
) -> Vec<BpfInsn> {
    const ALLOW: i32 = 1;
    let v4_off = if egress { IPV4_DST_OFF } else { IPV4_SRC_OFF };
    let v6_off = if egress { IPV6_DST_OFF } else { IPV6_SRC_OFF };

    let mut insns: Vec<BpfInsn> = Vec::new();
    let mut allow_patches: Vec<usize> = Vec::new();
    let mut default_patches: Vec<usize> = Vec::new();

    // Header: load bounds, ensure one byte, read the version nibble.
    insns.push(ldx_mem_w(R2, R1, SK_BUFF_DATA_OFF)); // r2 = data
    insns.push(ldx_mem_w(R3, R1, SK_BUFF_DATA_END_OFF)); // r3 = data_end
    insns.push(mov64_reg(R4, R2));
    insns.push(alu64_add_imm(R4, 1));
    allow_patches.push(insns.len());
    insns.push(jgt_reg(R4, R3, 0)); // < 1 byte -> ALLOW
    insns.push(ldx_mem_b(R5, R2, 0));
    insns.push(alu64_rsh_imm(R5, 4)); // r5 = version
    let to_v6 = insns.len();
    insns.push(jne32_imm(R5, 4, 0)); // not IPv4 -> IPv6 path (patched)

    // ── IPv4 path ── r6 = address; each prefix masks + compares.
    insns.push(mov64_reg(R4, R2));
    insns.push(alu64_add_imm(R4, 20));
    allow_patches.push(insns.len());
    insns.push(jgt_reg(R4, R3, 0)); // < 20 bytes -> ALLOW
    insns.push(ldx_mem_w(R6, R2, v4_off));
    for &(network, mask, verdict) in v4 {
        insns.push(mov64_reg(R7, R6));
        insns.push(alu32_and_imm(R7, mask as i32));
        insns.push(jne32_imm(R7, network as i32, 2)); // skip mov + exit
        insns.push(mov64_imm(R0, verdict));
        insns.push(exit_insn());
    }
    default_patches.push(insns.len());
    insns.push(insn(BPF_JA, 0, 0, 0, 0)); // -> DEFAULT (patched)

    // ── IPv6 path ── r6..r9 = the four address words; each prefix ANDs and
    // compares all four (any mismatch skips to the next prefix). r5 is the temp.
    let v6_start = insns.len();
    insns[to_v6].off = (v6_start - to_v6 - 1) as i16;
    allow_patches.push(insns.len());
    insns.push(jne32_imm(R5, 6, 0)); // not IPv6 either -> ALLOW
    insns.push(mov64_reg(R4, R2));
    insns.push(alu64_add_imm(R4, 40));
    allow_patches.push(insns.len());
    insns.push(jgt_reg(R4, R3, 0)); // < 40 bytes -> ALLOW
    insns.push(ldx_mem_w(R6, R2, v6_off));
    insns.push(ldx_mem_w(R7, R2, v6_off + 4));
    insns.push(ldx_mem_w(R8, R2, v6_off + 8));
    insns.push(ldx_mem_w(R9, R2, v6_off + 12));
    let words = [R6, R7, R8, R9];
    for &(network, mask, verdict) in v6 {
        let mut skips: Vec<usize> = Vec::new();
        for w in 0..4 {
            insns.push(mov64_reg(R5, words[w]));
            insns.push(alu32_and_imm(R5, mask[w] as i32));
            skips.push(insns.len());
            insns.push(jne32_imm(R5, network[w] as i32, 0)); // -> next prefix (patched)
        }
        insns.push(mov64_imm(R0, verdict));
        insns.push(exit_insn());
        let next = insns.len();
        for s in skips {
            insns[s].off = (next - s - 1) as i16;
        }
    }
    default_patches.push(insns.len());
    insns.push(insn(BPF_JA, 0, 0, 0, 0)); // -> DEFAULT (patched)

    // ── DEFAULT (unmatched, filtered family) then ALLOW (unfiltered) tails.
    let default_idx = insns.len();
    insns.push(mov64_imm(R0, default_verdict));
    insns.push(exit_insn());
    let allow_idx = insns.len();
    insns.push(mov64_imm(R0, ALLOW));
    insns.push(exit_insn());

    for p in default_patches {
        insns[p].off = (default_idx - p - 1) as i16;
    }
    for p in allow_patches {
        insns[p].off = (allow_idx - p - 1) as i16;
    }
    insns
}

/// Apply `IPAddressAllow=` / `IPAddressDeny=` by attaching cgroup/skb filters
/// (ingress checks the source address, egress the destination). Longest-prefix
/// wins, allow beats deny on ties; if any allow entry is present the default for
/// unmatched IPv4 traffic is deny, otherwise allow. A dropped packet's send
/// fails with EPERM. IPv6 is not yet filtered.
pub fn apply_ip_address_policy(
    cgroup_path: &Path,
    allow: &[String],
    deny: &[String],
) -> Result<(), String> {
    if allow.is_empty() && deny.is_empty() {
        return Ok(());
    }
    // (network, mask, prefixlen, is_allow) per family. A special name (localhost,
    // any, ...) resolves in both families, so one entry can feed both lists.
    let mut v4: Vec<(u32, u32, u8, bool)> = Vec::new();
    let mut v6: Vec<([u32; 4], [u32; 4], u8, bool)> = Vec::new();
    for (list, is_allow) in [(allow, true), (deny, false)] {
        for e in list {
            if let Some((net, mask, len)) = ipv4_prefix_from_entry(e) {
                v4.push((net, mask, len, is_allow));
            }
            if let Some((net, mask, len)) = ipv6_prefix_from_entry(e) {
                v6.push((net, mask, len, is_allow));
            }
        }
    }
    // Longest prefix first; allow before deny on a tie so allow wins.
    v4.sort_by(|a, b| b.2.cmp(&a.2).then(b.3.cmp(&a.3)));
    v6.sort_by(|a, b| b.2.cmp(&a.2).then(b.3.cmp(&a.3)));
    let v4s: Vec<(u32, u32, i32)> = v4
        .iter()
        .map(|&(net, mask, _len, is_allow)| (net, mask, if is_allow { 1 } else { 0 }))
        .collect();
    let v6s: Vec<([u32; 4], [u32; 4], i32)> = v6
        .iter()
        .map(|&(net, mask, _len, is_allow)| (net, mask, if is_allow { 1 } else { 0 }))
        .collect();
    // If any IPAddressAllow= is configured, unmatched traffic (of a filtered
    // family) is denied.
    let default_verdict: i32 = if allow.is_empty() { 1 } else { 0 };

    let egress = build_ip_filter_program(&v4s, &v6s, default_verdict, true);
    let ingress = build_ip_filter_program(&v4s, &v6s, default_verdict, false);

    let e_fd = bpf_prog_load(&egress, BPF_PROG_TYPE_CGROUP_SKB)?;
    let e_res = bpf_prog_attach(e_fd, cgroup_path, BPF_CGROUP_INET_EGRESS);
    unsafe {
        libc::close(e_fd);
    }
    e_res?;

    let i_fd = bpf_prog_load(&ingress, BPF_PROG_TYPE_CGROUP_SKB)?;
    let i_res = bpf_prog_attach(i_fd, cgroup_path, BPF_CGROUP_INET_INGRESS);
    unsafe {
        libc::close(i_fd);
    }
    i_res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ipv4_cidr_endianness() {
        // network+mask are in the byte order a little-endian packet load yields,
        // so a masked compare against the loaded address works directly.
        let (net, mask, len) = parse_ipv4_cidr("127.0.0.0/8").unwrap();
        assert_eq!((net, mask, len), (0x0000_007F, 0x0000_00FF, 8));
        // A packet to 127.0.0.1 loads as from_le([127,0,0,1]); it must match.
        assert_eq!(u32::from_le_bytes([127, 0, 0, 1]) & mask, net);
        assert_ne!(u32::from_le_bytes([10, 0, 0, 1]) & mask, net);

        assert_eq!(
            parse_ipv4_cidr("10.0.0.0/8").unwrap(),
            (0x0000_000A, 0x0000_00FF, 8)
        );
        let (n16, m16, _) = parse_ipv4_cidr("192.168.0.0/16").unwrap();
        assert_eq!(u32::from_le_bytes([192, 168, 5, 9]) & m16, n16);
        assert_ne!(u32::from_le_bytes([192, 169, 0, 0]) & m16, n16);

        // A bare address is /32; special names resolve; IPv6 is skipped.
        assert_eq!(parse_ipv4_cidr("8.8.8.8").unwrap().2, 32);
        assert_eq!(ipv4_prefix_from_entry("any"), Some((0, 0, 0)));
        assert_eq!(ipv4_prefix_from_entry("localhost").unwrap().2, 8);
        assert!(ipv4_prefix_from_entry("::1").is_none());
        assert!(ipv4_prefix_from_entry("fe80::/64").is_none());
    }

    /// Load 16 address octets as the four little-endian words a BPF `LDX_W`
    /// produces (what the program compares against).
    fn ipv6_words_le(octets: &[u8; 16]) -> [u32; 4] {
        let mut w = [0u32; 4];
        for (i, word) in w.iter_mut().enumerate() {
            *word = u32::from_le_bytes([
                octets[i * 4],
                octets[i * 4 + 1],
                octets[i * 4 + 2],
                octets[i * 4 + 3],
            ]);
        }
        w
    }

    #[test]
    fn test_parse_ipv6_cidr_endianness() {
        let (net, mask, len) = parse_ipv6_cidr("::1/128").unwrap();
        assert_eq!(len, 128);
        // A packet to ::1 loaded as four LE words must match ::1/128.
        let lo = ipv6_words_le(&std::net::Ipv6Addr::LOCALHOST.octets());
        assert!((0..4).all(|i| lo[i] & mask[i] == net[i]));
        let other = ipv6_words_le(&"2001:db8::1".parse::<std::net::Ipv6Addr>().unwrap().octets());
        assert!((0..4).any(|i| other[i] & mask[i] != net[i]));

        // fe80::/64 matches any fe80:: address on the high 64 bits only.
        let (n64, m64, l64) = parse_ipv6_cidr("fe80::/64").unwrap();
        assert_eq!(l64, 64);
        let ll = ipv6_words_le(&"fe80::abcd:1".parse::<std::net::Ipv6Addr>().unwrap().octets());
        assert!((0..4).all(|i| ll[i] & m64[i] == n64[i]));
        let g = ipv6_words_le(&"2001:db8::1".parse::<std::net::Ipv6Addr>().unwrap().octets());
        assert!(g[0] & m64[0] != n64[0]);

        // Special names resolve to their IPv6 halves; bare IPv4 has no v6 prefix.
        assert_eq!(ipv6_prefix_from_entry("any"), Some(([0; 4], [0; 4], 0)));
        assert_eq!(ipv6_prefix_from_entry("localhost").unwrap().2, 128);
        assert_eq!(ipv6_prefix_from_entry("link-local").unwrap().2, 64);
        assert!(ipv6_prefix_from_entry("127.0.0.0/8").is_none());
        assert!(ipv6_prefix_from_entry("10.0.0.0/8").is_none());
    }
}
