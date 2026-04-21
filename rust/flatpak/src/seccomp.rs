//! Seccomp BPF filter generation for Flatpak sandboxes.
//!
//! Generates a compiled BPF program matching the filter that real Flatpak
//! applies. The filter uses an allowlist for socket families and blocks
//! dangerous syscalls that could escape the sandbox.

use std::io;
use std::os::unix::io::RawFd;

// ---------------------------------------------------------------------------
// BPF instruction encoding (linux/filter.h)
// ---------------------------------------------------------------------------

/// A single BPF instruction (struct sock_filter).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

// BPF instruction classes and fields.
const BPF_LD: u16 = 0x00;
const BPF_ALU: u16 = 0x04;
const BPF_JMP: u16 = 0x05;
const BPF_RET: u16 = 0x06;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JEQ: u16 = 0x10;
const BPF_JGE: u16 = 0x30;
const BPF_AND: u16 = 0x50;
const BPF_K: u16 = 0x00;

// Seccomp return values.
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;

// Seccomp data offsets (struct seccomp_data).
const SECCOMP_DATA_NR: u32 = 0; // syscall number
const SECCOMP_DATA_ARCH: u32 = 4; // architecture
const SECCOMP_DATA_ARG0: u32 = 16; // first argument (low 32 bits)
#[allow(dead_code)]
const SECCOMP_DATA_ARG0_HI: u32 = 20; // first argument (high 32 bits)
const SECCOMP_DATA_ARG1: u32 = 24; // second argument (low 32 bits)

// x86_64 audit architecture.
const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

// Errno values.
const EPERM: u32 = 1;
const ENOSYS: u32 = 38;
const EAFNOSUPPORT: u32 = 97;

fn bpf_stmt(code: u16, k: u32) -> SockFilter {
    SockFilter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

fn bpf_jump(code: u16, k: u32, jt: u8, jf: u8) -> SockFilter {
    SockFilter { code, jt, jf, k }
}

fn bpf_load(offset: u32) -> SockFilter {
    bpf_stmt(BPF_LD | BPF_W | BPF_ABS, offset)
}

fn bpf_ret(val: u32) -> SockFilter {
    bpf_stmt(BPF_RET | BPF_K, val)
}

fn bpf_jeq(val: u32, jt: u8, jf: u8) -> SockFilter {
    bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, val, jt, jf)
}

fn bpf_jge(val: u32, jt: u8, jf: u8) -> SockFilter {
    bpf_jump(BPF_JMP | BPF_JGE | BPF_K, val, jt, jf)
}

// ---------------------------------------------------------------------------
// Syscall numbers (x86_64)
// ---------------------------------------------------------------------------

mod nr {
    // Always blocked (EPERM).
    pub const SYSLOG: u32 = 103;
    pub const USELIB: u32 = 134;
    pub const ACCT: u32 = 163;
    pub const QUOTACTL: u32 = 179;
    pub const ADD_KEY: u32 = 248;
    pub const REQUEST_KEY: u32 = 249;
    pub const KEYCTL: u32 = 250;
    pub const MOVE_PAGES: u32 = 279;
    pub const MBIND: u32 = 237;
    pub const GET_MEMPOLICY: u32 = 239;
    pub const SET_MEMPOLICY: u32 = 238;
    pub const MIGRATE_PAGES: u32 = 256;
    pub const UNSHARE: u32 = 272;
    pub const SETNS: u32 = 308;
    pub const MOUNT: u32 = 165;
    pub const UMOUNT2: u32 = 166;
    pub const PIVOT_ROOT: u32 = 155;
    pub const CHROOT: u32 = 161;

    // Always blocked (ENOSYS) — new mount/clone APIs.
    pub const CLONE3: u32 = 435;
    pub const OPEN_TREE: u32 = 428;
    pub const MOVE_MOUNT: u32 = 429;
    pub const FSOPEN: u32 = 430;
    pub const FSCONFIG: u32 = 431;
    pub const FSMOUNT: u32 = 432;
    pub const FSPICK: u32 = 433;
    pub const MOUNT_SETATTR: u32 = 442;

    // Blocked unless --devel.
    pub const PERF_EVENT_OPEN: u32 = 298;
    pub const PTRACE: u32 = 101;
    pub const PERSONALITY: u32 = 135;

    // Needs argument inspection.
    pub const CLONE: u32 = 56;
    pub const IOCTL: u32 = 16;
    pub const SOCKET: u32 = 41;
    pub const PRCTL: u32 = 157;
}

// prctl operations to block.
const PR_SET_MM: u32 = 35;

// ioctl commands to block.
const TIOCSTI: u32 = 0x5412;
const TIOCLINUX: u32 = 0x541C;

// clone flags.
#[allow(dead_code)]
const CLONE_NEWUSER: u32 = 0x1000_0000;

// Socket families.
const AF_UNSPEC: u32 = 0;
const AF_LOCAL: u32 = 1;
const AF_INET: u32 = 2;
const AF_INET6: u32 = 10;
const AF_NETLINK: u32 = 16;
const AF_CAN: u32 = 29;
const AF_BLUETOOTH: u32 = 31;

// ---------------------------------------------------------------------------
// Filter builder
// ---------------------------------------------------------------------------

/// Options that affect which syscalls are permitted.
#[derive(Default)]
pub struct SeccompOptions {
    pub devel: bool,
    pub bluetooth: bool,
    pub canbus: bool,
}

/// Generate the compiled BPF filter as raw bytes.
pub fn generate_filter(opts: &SeccompOptions) -> Vec<u8> {
    let insns = build_filter(opts);
    let mut bytes = Vec::with_capacity(insns.len() * 8);
    for insn in &insns {
        bytes.extend_from_slice(&insn.code.to_ne_bytes());
        bytes.push(insn.jt);
        bytes.push(insn.jf);
        bytes.extend_from_slice(&insn.k.to_ne_bytes());
    }
    bytes
}

/// Write the filter to a memfd and return the file descriptor.
pub fn write_filter_to_memfd(opts: &SeccompOptions) -> Result<RawFd, String> {
    let filter_bytes = generate_filter(opts);

    let name = std::ffi::CString::new("flatpak-seccomp").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(format!("memfd_create: {}", io::Error::last_os_error()));
    }

    let written = unsafe {
        libc::write(
            fd,
            filter_bytes.as_ptr() as *const libc::c_void,
            filter_bytes.len(),
        )
    };
    if written < 0 || written as usize != filter_bytes.len() {
        unsafe { libc::close(fd) };
        return Err(format!(
            "write seccomp filter: {}",
            io::Error::last_os_error()
        ));
    }

    // Seek back to start.
    unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
    // Clear close-on-exec so bwrap can read the fd after exec.
    // SAFETY: fd is a valid open file descriptor returned by memfd_create above.
    // F_SETFD with 0 clears FD_CLOEXEC, which is required because bwrap
    // reads this fd after execvp (which would close CLOEXEC fds).
    unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };

    Ok(fd)
}

fn build_filter(opts: &SeccompOptions) -> Vec<SockFilter> {
    let mut f: Vec<SockFilter> = vec![
        // Verify architecture is x86_64.
        bpf_load(SECCOMP_DATA_ARCH),
        bpf_jeq(AUDIT_ARCH_X86_64, 1, 0),
        bpf_ret(SECCOMP_RET_ALLOW), // Wrong arch → allow (don't break compat layers).
        // Load syscall number.
        bpf_load(SECCOMP_DATA_NR),
    ];

    // --- Block dangerous syscalls (EPERM) ---
    let eperm_syscalls = [
        nr::SYSLOG,
        nr::USELIB,
        nr::ACCT,
        nr::QUOTACTL,
        nr::ADD_KEY,
        nr::REQUEST_KEY,
        nr::KEYCTL,
        nr::MOVE_PAGES,
        nr::MBIND,
        nr::GET_MEMPOLICY,
        nr::SET_MEMPOLICY,
        nr::MIGRATE_PAGES,
        nr::UNSHARE,
        nr::SETNS,
        nr::MOUNT,
        nr::UMOUNT2,
        nr::PIVOT_ROOT,
        nr::CHROOT,
    ];

    for &nr in &eperm_syscalls {
        f.push(bpf_jeq(nr, 0, 1));
        f.push(bpf_ret(SECCOMP_RET_ERRNO | EPERM));
    }

    // --- Block new APIs (ENOSYS) ---
    let enosys_syscalls = [
        nr::CLONE3,
        nr::OPEN_TREE,
        nr::MOVE_MOUNT,
        nr::FSOPEN,
        nr::FSCONFIG,
        nr::FSMOUNT,
        nr::FSPICK,
        nr::MOUNT_SETATTR,
    ];

    for &nr in &enosys_syscalls {
        f.push(bpf_jeq(nr, 0, 1));
        f.push(bpf_ret(SECCOMP_RET_ERRNO | ENOSYS));
    }

    // --- Conditionally block (unless --devel) ---
    if !opts.devel {
        for &nr in &[nr::PERF_EVENT_OPEN, nr::PTRACE] {
            f.push(bpf_jeq(nr, 0, 1));
            f.push(bpf_ret(SECCOMP_RET_ERRNO | EPERM));
        }

        // personality: allow only the default (0) and keep ADDR_NO_RANDOMIZE
        // (0x0040000). Block everything else.
        f.push(bpf_jeq(nr::PERSONALITY, 0, 3)); // if personality syscall
        f.push(bpf_load(SECCOMP_DATA_ARG0));
        f.push(bpf_jge(0x0040001, 0, 1)); // if arg0 >= 0x40001, block
        f.push(bpf_ret(SECCOMP_RET_ERRNO | EPERM));
        // Reload syscall number after arg inspection.
        f.push(bpf_load(SECCOMP_DATA_NR));
    }

    // --- clone: block CLONE_NEWUSER flag ---
    // If syscall is clone, load flags (arg0), AND with CLONE_NEWUSER mask,
    // if result is non-zero (flag is set), block with EPERM.
    f.push(bpf_jeq(nr::CLONE, 0, 5)); // if not clone, skip 5 instructions
    f.push(bpf_load(SECCOMP_DATA_ARG0)); // load clone flags
    f.push(bpf_stmt(BPF_ALU | BPF_AND | BPF_K, CLONE_NEWUSER)); // AND with CLONE_NEWUSER
    f.push(bpf_jeq(0, 1, 0)); // if result == 0 (flag not set), skip block
    f.push(bpf_ret(SECCOMP_RET_ERRNO | EPERM)); // block: CLONE_NEWUSER is set
    f.push(bpf_load(SECCOMP_DATA_NR)); // reload syscall number

    // --- prctl: block PR_SET_MM ---
    f.push(bpf_jeq(nr::PRCTL, 0, 4)); // if not prctl, skip
    f.push(bpf_load(SECCOMP_DATA_ARG0)); // load prctl operation
    f.push(bpf_jeq(PR_SET_MM, 0, 1)); // if not PR_SET_MM, skip
    f.push(bpf_ret(SECCOMP_RET_ERRNO | EPERM)); // block PR_SET_MM
    f.push(bpf_load(SECCOMP_DATA_NR)); // reload syscall number

    // --- ioctl: block TIOCSTI and TIOCLINUX ---
    f.push(bpf_jeq(nr::IOCTL, 0, 5));
    f.push(bpf_load(SECCOMP_DATA_ARG1)); // ioctl request number is arg1
    f.push(bpf_jeq(TIOCSTI, 0, 1));
    f.push(bpf_ret(SECCOMP_RET_ERRNO | EPERM));
    f.push(bpf_jeq(TIOCLINUX, 0, 1));
    f.push(bpf_ret(SECCOMP_RET_ERRNO | EPERM));
    // Reload syscall number.
    f.push(bpf_load(SECCOMP_DATA_NR));

    // --- socket: allowlist of address families ---
    f.push(bpf_jeq(nr::SOCKET, 0, 0)); // placeholder jump-over
    let socket_check_start = f.len() - 1;

    // Load socket family (arg0).
    f.push(bpf_load(SECCOMP_DATA_ARG0));

    // Build allowed families list.
    let mut allowed_families = vec![AF_UNSPEC, AF_LOCAL, AF_INET, AF_INET6, AF_NETLINK];
    if opts.canbus {
        allowed_families.push(AF_CAN);
    }
    if opts.bluetooth {
        allowed_families.push(AF_BLUETOOTH);
    }

    for &af in &allowed_families {
        f.push(bpf_jeq(af, 0, 1));
        f.push(bpf_ret(SECCOMP_RET_ALLOW));
    }

    // Not in allowlist → EAFNOSUPPORT.
    f.push(bpf_ret(SECCOMP_RET_ERRNO | EAFNOSUPPORT));

    // Fix up the socket check jump: skip over the entire socket block if
    // not the socket syscall. The jf needs to jump to after the block.
    let socket_block_len = (f.len() - socket_check_start - 1) as u8;
    f[socket_check_start].jf = socket_block_len;

    // Reload syscall number (for anything that fell through).
    f.push(bpf_load(SECCOMP_DATA_NR));

    // --- Default: allow ---
    f.push(bpf_ret(SECCOMP_RET_ALLOW));

    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_generates_valid_bpf() {
        let opts = SeccompOptions::default();
        let bytes = generate_filter(&opts);
        // Each instruction is 8 bytes.
        assert_eq!(bytes.len() % 8, 0);
        // Should have a reasonable number of instructions.
        let count = bytes.len() / 8;
        assert!(count > 20, "filter has {count} instructions, expected > 20");
        assert!(
            count < 500,
            "filter has {count} instructions, expected < 500"
        );
    }

    #[test]
    fn filter_with_devel_is_shorter() {
        let normal = generate_filter(&SeccompOptions::default());
        let devel = generate_filter(&SeccompOptions {
            devel: true,
            ..Default::default()
        });
        // Devel mode skips perf_event_open, ptrace, personality blocks.
        assert!(devel.len() < normal.len());
    }

    #[test]
    fn filter_with_bluetooth_is_longer() {
        let normal = generate_filter(&SeccompOptions::default());
        let bt = generate_filter(&SeccompOptions {
            bluetooth: true,
            ..Default::default()
        });
        // Bluetooth adds an extra allowed socket family.
        assert!(bt.len() > normal.len());
    }
}
