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

fn bpf_prog_load(insns: &[BpfInsn]) -> Result<i32, String> {
    let license = b"GPL\0";
    // Use a zeroed buffer large enough for the bpf_attr union.
    let mut attr = [0u8; 128];

    // prog_type at offset 0
    attr[0..4].copy_from_slice(&BPF_PROG_TYPE_CGROUP_DEVICE.to_ne_bytes());
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

fn bpf_prog_attach(prog_fd: i32, cgroup_path: &Path) -> Result<(), String> {
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
    attr[8..12].copy_from_slice(&BPF_CGROUP_DEVICE.to_ne_bytes());
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

    let prog_fd = bpf_prog_load(&program)?;
    let result = bpf_prog_attach(prog_fd, cgroup_path);

    // Close the program fd — the kernel keeps its own reference after attach.
    unsafe {
        libc::close(prog_fd);
    }

    result
}
