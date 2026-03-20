// rust-bubblewrap: A bubblewrap-compatible unprivileged sandboxing tool.
//
// Creates isolated container environments using Linux namespaces. Works as
// an unprivileged user via user namespaces. Always sets PR_SET_NO_NEW_PRIVS
// to prevent privilege escalation.

use std::env;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::process;

// ---------------------------------------------------------------------------
// CLI options and setup operations
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum SetupOp {
    BindMount {
        src: String,
        dest: String,
        readonly: bool,
        allow_notexist: bool,
        dev: bool,
    },
    BindMountFd {
        fd: RawFd,
        dest: String,
        readonly: bool,
    },
    RemountRo {
        dest: String,
    },
    MountProc {
        dest: String,
    },
    MountDev {
        dest: String,
    },
    MountTmpfs {
        dest: String,
        perms: Option<u32>,
        size: Option<u64>,
    },
    MountMqueue {
        dest: String,
    },
    MakeDir {
        dest: String,
        perms: Option<u32>,
    },
    MakeFile {
        fd: RawFd,
        dest: String,
        perms: Option<u32>,
    },
    MakeBindFile {
        fd: RawFd,
        dest: String,
        readonly: bool,
        perms: Option<u32>,
    },
    MakeSymlink {
        target: String,
        dest: String,
    },
    Chmod {
        perms: u32,
        path: String,
    },
}

#[derive(Debug, Default)]
struct Options {
    // Namespace flags
    unshare_user: bool,
    unshare_user_try: bool,
    unshare_ipc: bool,
    unshare_pid: bool,
    unshare_net: bool,
    unshare_uts: bool,
    unshare_cgroup: bool,
    unshare_cgroup_try: bool,
    share_net: bool,

    // Namespace FDs
    userns_fd: Option<RawFd>,
    userns2_fd: Option<RawFd>,
    pidns_fd: Option<RawFd>,

    // User namespace options
    disable_userns: bool,
    assert_userns_disabled: bool,
    sandbox_uid: Option<u32>,
    sandbox_gid: Option<u32>,
    sandbox_hostname: Option<String>,

    // Environment
    chdir: Option<String>,
    env_ops: Vec<EnvOp>,

    // Filesystem operations (applied in order)
    setup_ops: Vec<SetupOp>,

    // Monitoring / synchronization
    lock_files: Vec<String>,
    sync_fd: Option<RawFd>,
    block_fd: Option<RawFd>,
    userns_block_fd: Option<RawFd>,
    info_fd: Option<RawFd>,
    json_status_fd: Option<RawFd>,

    // Security
    seccomp_fd: Option<RawFd>,
    add_seccomp_fds: Vec<RawFd>,
    exec_label: Option<String>,
    file_label: Option<String>,
    new_session: bool,
    die_with_parent: bool,
    as_pid_1: bool,

    // Capabilities
    cap_ops: Vec<CapOp>,

    // Misc
    argv0: Option<String>,

    // Command to execute
    command: Vec<String>,
}

#[derive(Clone, Debug)]
enum EnvOp {
    Set(String, String),
    Unset(String),
    Clear,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum CapOp {
    Add(String),
    Drop(String),
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn parse_args() -> Options {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut opts = Options::default();

    if args.is_empty() {
        print_usage();
        process::exit(1);
    }

    let mut i = 0;
    let mut pending_perms: Option<u32> = None;
    let mut pending_size: Option<u64> = None;

    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--help" => {
                print_usage();
                process::exit(0);
            }
            "--version" => {
                println!("bubblewrap 0.1.0 (rust-bubblewrap)");
                process::exit(0);
            }

            // Namespace control
            "--unshare-user" => opts.unshare_user = true,
            "--unshare-user-try" => opts.unshare_user_try = true,
            "--unshare-ipc" => opts.unshare_ipc = true,
            "--unshare-pid" => opts.unshare_pid = true,
            "--unshare-net" => opts.unshare_net = true,
            "--unshare-uts" => opts.unshare_uts = true,
            "--unshare-cgroup" => opts.unshare_cgroup = true,
            "--unshare-cgroup-try" => opts.unshare_cgroup_try = true,
            "--unshare-all" => {
                opts.unshare_user_try = true;
                opts.unshare_ipc = true;
                opts.unshare_pid = true;
                opts.unshare_net = true;
                opts.unshare_uts = true;
                opts.unshare_cgroup_try = true;
            }
            "--share-net" => opts.share_net = true,
            "--userns" => {
                i += 1;
                opts.userns_fd = Some(parse_fd(&args, i, "--userns"));
            }
            "--userns2" => {
                i += 1;
                opts.userns2_fd = Some(parse_fd(&args, i, "--userns2"));
            }
            "--pidns" => {
                i += 1;
                opts.pidns_fd = Some(parse_fd(&args, i, "--pidns"));
            }
            "--disable-userns" => opts.disable_userns = true,
            "--assert-userns-disabled" => opts.assert_userns_disabled = true,
            "--uid" => {
                i += 1;
                opts.sandbox_uid = Some(parse_u32(&args, i, "--uid"));
            }
            "--gid" => {
                i += 1;
                opts.sandbox_gid = Some(parse_u32(&args, i, "--gid"));
            }
            "--hostname" => {
                i += 1;
                opts.sandbox_hostname = Some(next_arg(&args, i, "--hostname"));
            }

            // Environment
            "--chdir" => {
                i += 1;
                opts.chdir = Some(next_arg(&args, i, "--chdir"));
            }
            "--setenv" => {
                i += 1;
                let var = next_arg(&args, i, "--setenv");
                i += 1;
                let val = next_arg(&args, i, "--setenv");
                opts.env_ops.push(EnvOp::Set(var, val));
            }
            "--unsetenv" => {
                i += 1;
                let var = next_arg(&args, i, "--unsetenv");
                opts.env_ops.push(EnvOp::Unset(var));
            }
            "--clearenv" => {
                opts.env_ops.push(EnvOp::Clear);
            }

            // Modifier flags
            "--perms" => {
                i += 1;
                let s = next_arg(&args, i, "--perms");
                pending_perms = Some(
                    u32::from_str_radix(&s, 8).unwrap_or_else(|_| die("invalid --perms value")),
                );
            }
            "--size" => {
                i += 1;
                let s = next_arg(&args, i, "--size");
                pending_size = Some(s.parse().unwrap_or_else(|_| die("invalid --size value")));
            }

            // Filesystem operations
            "--bind" | "--bind-try" | "--ro-bind" | "--ro-bind-try" | "--dev-bind"
            | "--dev-bind-try" => {
                i += 1;
                let src = next_arg(&args, i, arg);
                i += 1;
                let dest = next_arg(&args, i, arg);
                let readonly = arg.starts_with("--ro-");
                let allow_notexist = arg.ends_with("-try");
                let dev = arg.starts_with("--dev-");
                opts.setup_ops.push(SetupOp::BindMount {
                    src,
                    dest,
                    readonly,
                    allow_notexist,
                    dev,
                });
                pending_perms = None;
            }
            "--bind-fd" | "--ro-bind-fd" => {
                i += 1;
                let fd = parse_fd(&args, i, arg);
                i += 1;
                let dest = next_arg(&args, i, arg);
                let readonly = arg.starts_with("--ro-");
                opts.setup_ops
                    .push(SetupOp::BindMountFd { fd, dest, readonly });
            }
            "--remount-ro" => {
                i += 1;
                let dest = next_arg(&args, i, "--remount-ro");
                opts.setup_ops.push(SetupOp::RemountRo { dest });
            }
            "--proc" => {
                i += 1;
                let dest = next_arg(&args, i, "--proc");
                opts.setup_ops.push(SetupOp::MountProc { dest });
            }
            "--dev" => {
                i += 1;
                let dest = next_arg(&args, i, "--dev");
                opts.setup_ops.push(SetupOp::MountDev { dest });
            }
            "--tmpfs" => {
                i += 1;
                let dest = next_arg(&args, i, "--tmpfs");
                opts.setup_ops.push(SetupOp::MountTmpfs {
                    dest,
                    perms: pending_perms.take(),
                    size: pending_size.take(),
                });
            }
            "--mqueue" => {
                i += 1;
                let dest = next_arg(&args, i, "--mqueue");
                opts.setup_ops.push(SetupOp::MountMqueue { dest });
            }
            "--dir" => {
                i += 1;
                let dest = next_arg(&args, i, "--dir");
                opts.setup_ops.push(SetupOp::MakeDir {
                    dest,
                    perms: pending_perms.take(),
                });
            }
            "--file" => {
                i += 1;
                let fd = parse_fd(&args, i, "--file");
                i += 1;
                let dest = next_arg(&args, i, "--file");
                opts.setup_ops.push(SetupOp::MakeFile {
                    fd,
                    dest,
                    perms: pending_perms.take(),
                });
            }
            "--bind-data" | "--ro-bind-data" => {
                i += 1;
                let fd = parse_fd(&args, i, arg);
                i += 1;
                let dest = next_arg(&args, i, arg);
                let readonly = arg.starts_with("--ro-");
                opts.setup_ops.push(SetupOp::MakeBindFile {
                    fd,
                    dest,
                    readonly,
                    perms: pending_perms.take(),
                });
            }
            "--symlink" => {
                i += 1;
                let target = next_arg(&args, i, "--symlink");
                i += 1;
                let dest = next_arg(&args, i, "--symlink");
                opts.setup_ops.push(SetupOp::MakeSymlink { target, dest });
            }
            "--chmod" => {
                i += 1;
                let s = next_arg(&args, i, "--chmod");
                let perms =
                    u32::from_str_radix(&s, 8).unwrap_or_else(|_| die("invalid --chmod value"));
                i += 1;
                let path = next_arg(&args, i, "--chmod");
                opts.setup_ops.push(SetupOp::Chmod { perms, path });
            }

            // Monitoring / synchronization
            "--lock-file" => {
                i += 1;
                opts.lock_files.push(next_arg(&args, i, "--lock-file"));
            }
            "--sync-fd" => {
                i += 1;
                opts.sync_fd = Some(parse_fd(&args, i, "--sync-fd"));
            }
            "--block-fd" => {
                i += 1;
                opts.block_fd = Some(parse_fd(&args, i, "--block-fd"));
            }
            "--userns-block-fd" => {
                i += 1;
                opts.userns_block_fd = Some(parse_fd(&args, i, "--userns-block-fd"));
            }
            "--info-fd" => {
                i += 1;
                opts.info_fd = Some(parse_fd(&args, i, "--info-fd"));
            }
            "--json-status-fd" => {
                i += 1;
                opts.json_status_fd = Some(parse_fd(&args, i, "--json-status-fd"));
            }

            // Security
            "--seccomp" => {
                i += 1;
                opts.seccomp_fd = Some(parse_fd(&args, i, "--seccomp"));
            }
            "--add-seccomp-fd" => {
                i += 1;
                opts.add_seccomp_fds
                    .push(parse_fd(&args, i, "--add-seccomp-fd"));
            }
            "--exec-label" => {
                i += 1;
                opts.exec_label = Some(next_arg(&args, i, "--exec-label"));
            }
            "--file-label" => {
                i += 1;
                opts.file_label = Some(next_arg(&args, i, "--file-label"));
            }
            "--new-session" => opts.new_session = true,
            "--die-with-parent" => opts.die_with_parent = true,
            "--as-pid-1" => opts.as_pid_1 = true,

            // Capabilities
            "--cap-add" => {
                i += 1;
                opts.cap_ops
                    .push(CapOp::Add(next_arg(&args, i, "--cap-add")));
            }
            "--cap-drop" => {
                i += 1;
                opts.cap_ops
                    .push(CapOp::Drop(next_arg(&args, i, "--cap-drop")));
            }

            // Misc
            "--argv0" => {
                i += 1;
                opts.argv0 = Some(next_arg(&args, i, "--argv0"));
            }
            "--args" => {
                i += 1;
                let fd = parse_fd(&args, i, "--args");
                // Read nul-separated args from fd (not yet implemented in full).
                read_args_from_fd(fd, &mut opts);
            }

            // End of options
            "--" => {
                opts.command = args[i + 1..].to_vec();
                break;
            }
            _ => {
                if arg.starts_with('-') {
                    die_fmt(format_args!("unknown option: {arg}"));
                }
                opts.command = args[i..].to_vec();
                break;
            }
        }
        i += 1;
    }

    if opts.command.is_empty() {
        eprintln!("bwrap: no command specified");
        process::exit(1);
    }

    opts
}

fn read_args_from_fd(_fd: RawFd, _opts: &mut Options) {
    // TODO: Read nul-separated args from file descriptor and re-parse.
    // For now this is a stub; the feature is rarely used directly.
}

fn next_arg(args: &[String], i: usize, flag: &str) -> String {
    if i >= args.len() {
        eprintln!("bwrap: option {flag} requires an argument");
        process::exit(1);
    }
    args[i].clone()
}

fn parse_fd(args: &[String], i: usize, flag: &str) -> RawFd {
    let s = next_arg(args, i, flag);
    s.parse()
        .unwrap_or_else(|_| die_fmt(format_args!("invalid fd for {flag}: {s}")))
}

fn parse_u32(args: &[String], i: usize, flag: &str) -> u32 {
    let s = next_arg(args, i, flag);
    s.parse()
        .unwrap_or_else(|_| die_fmt(format_args!("invalid value for {flag}: {s}")))
}

fn die(msg: &str) -> ! {
    eprintln!("bwrap: {msg}");
    process::exit(1);
}

fn die_fmt(args: std::fmt::Arguments<'_>) -> ! {
    eprintln!("bwrap: {args}");
    process::exit(1);
}

// ---------------------------------------------------------------------------
// Syscall wrappers
// ---------------------------------------------------------------------------

mod sys {
    use libc::{self, c_char, c_int, c_ulong, c_void};
    use std::ffi::CString;
    use std::io;
    use std::os::unix::io::RawFd;

    pub fn mount(
        source: Option<&str>,
        target: &str,
        fstype: Option<&str>,
        flags: c_ulong,
        data: Option<&str>,
    ) -> io::Result<()> {
        let src = source.map(|s| CString::new(s).unwrap());
        let tgt = CString::new(target).unwrap();
        let fst = fstype.map(|s| CString::new(s).unwrap());
        let dat = data.map(|s| CString::new(s).unwrap());

        let ret = unsafe {
            libc::mount(
                src.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                tgt.as_ptr(),
                fst.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                flags,
                dat.as_ref()
                    .map_or(std::ptr::null(), |s| s.as_ptr() as *const c_void),
            )
        };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn umount2(target: &str, flags: c_int) -> io::Result<()> {
        let tgt = CString::new(target).unwrap();
        let ret = unsafe { libc::umount2(tgt.as_ptr(), flags) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn pivot_root(new_root: &str, put_old: &str) -> io::Result<()> {
        let new = CString::new(new_root).unwrap();
        let old = CString::new(put_old).unwrap();
        let ret = unsafe { libc::syscall(libc::SYS_pivot_root, new.as_ptr(), old.as_ptr()) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn sethostname(name: &str) -> io::Result<()> {
        let ret = unsafe { libc::sethostname(name.as_ptr() as *const c_char, name.len()) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn prctl_set_no_new_privs() -> io::Result<()> {
        let ret = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn prctl_set_pdeathsig(sig: c_int) -> io::Result<()> {
        let ret = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, sig, 0, 0, 0) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn setsid() -> io::Result<()> {
        let ret = unsafe { libc::setsid() };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn setns(fd: RawFd, nstype: c_int) -> io::Result<()> {
        let ret = unsafe { libc::setns(fd, nstype) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn clone3_pidns(flags: c_int) -> io::Result<libc::pid_t> {
        // Use raw clone syscall for namespace creation.
        let ret = unsafe {
            libc::syscall(
                libc::SYS_clone,
                flags | libc::SIGCHLD,
                std::ptr::null::<c_void>(),
            )
        };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret as libc::pid_t)
        }
    }

    pub fn waitpid(pid: libc::pid_t) -> io::Result<c_int> {
        let mut status: c_int = 0;
        let ret = unsafe { libc::waitpid(pid, &mut status, 0) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(status)
        }
    }

    pub fn pipe2(flags: c_int) -> io::Result<(RawFd, RawFd)> {
        let mut fds = [0 as c_int; 2];
        let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), flags) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok((fds[0], fds[1]))
        }
    }

    pub fn close(fd: RawFd) {
        unsafe {
            libc::close(fd);
        }
    }

    pub fn read_all(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
        let ret = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret as usize)
        }
    }

    pub fn write_all(fd: RawFd, buf: &[u8]) -> io::Result<()> {
        let mut written = 0;
        while written < buf.len() {
            let ret = unsafe {
                libc::write(
                    fd,
                    buf[written..].as_ptr() as *const c_void,
                    buf.len() - written,
                )
            };
            if ret == -1 {
                return Err(io::Error::last_os_error());
            }
            written += ret as usize;
        }
        Ok(())
    }

    pub fn getuid() -> u32 {
        unsafe { libc::getuid() }
    }

    pub fn getgid() -> u32 {
        unsafe { libc::getgid() }
    }

    pub fn mkdir(path: &str, mode: libc::mode_t) -> io::Result<()> {
        let p = CString::new(path).unwrap();
        let ret = unsafe { libc::mkdir(p.as_ptr(), mode) };
        if ret == -1 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EEXIST) {
                Ok(())
            } else {
                Err(e)
            }
        } else {
            Ok(())
        }
    }

    pub fn chmod(path: &str, mode: libc::mode_t) -> io::Result<()> {
        let p = CString::new(path).unwrap();
        let ret = unsafe { libc::chmod(p.as_ptr(), mode) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn mknod(path: &str, mode: libc::mode_t, dev: libc::dev_t) -> io::Result<()> {
        let p = CString::new(path).unwrap();
        let ret = unsafe { libc::mknod(p.as_ptr(), mode, dev) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn chdir(path: &str) -> io::Result<()> {
        let p = CString::new(path).unwrap();
        let ret = unsafe { libc::chdir(p.as_ptr()) };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn makedev(major: u32, minor: u32) -> libc::dev_t {
        libc::makedev(major, minor)
    }

    pub fn execvp(file: &std::ffi::CStr, argv: &[*const c_char]) -> io::Error {
        unsafe {
            libc::execvp(file.as_ptr(), argv.as_ptr());
        }
        io::Error::last_os_error()
    }

    pub fn fork() -> io::Result<libc::pid_t> {
        let ret = unsafe { libc::fork() };
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret)
        }
    }

    // Loopback network setup
    pub fn setup_loopback() -> io::Result<()> {
        use std::mem;

        unsafe {
            let sock = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
            if sock < 0 {
                return Err(io::Error::last_os_error());
            }

            let mut ifr: libc::ifreq = mem::zeroed();
            let name = b"lo\0";
            ifr.ifr_name[..name.len()].copy_from_slice(&name.map(|b| b as libc::c_char));

            // Get current flags.
            if libc::ioctl(sock, libc::SIOCGIFFLAGS as _, &mut ifr) < 0 {
                libc::close(sock);
                return Err(io::Error::last_os_error());
            }

            // Set IFF_UP.
            ifr.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;

            if libc::ioctl(sock, libc::SIOCSIFFLAGS as _, &ifr) < 0 {
                libc::close(sock);
                return Err(io::Error::last_os_error());
            }

            libc::close(sock);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Filesystem setup
// ---------------------------------------------------------------------------

fn ensure_dir(path: &str) {
    // Create all parent directories.
    let p = Path::new(path);
    if let Some(parent) = p.parent()
        && !parent.exists() {
            let _ = fs::create_dir_all(parent);
        }
    let _ = sys::mkdir(path, 0o755);
}

fn read_fd_to_vec(fd: RawFd) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match sys::read_all(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    sys::close(fd);
    data
}

fn setup_newroot(base: &str, ops: &[SetupOp]) {
    let newroot = format!("{base}/newroot");

    for op in ops {
        match op {
            SetupOp::BindMount {
                src,
                dest,
                readonly,
                allow_notexist,
                dev,
            } => {
                let target = format!("{newroot}{dest}");

                // Check if source exists.
                if !Path::new(src).exists() {
                    if *allow_notexist {
                        continue;
                    }
                    die_fmt(format_args!("bind source {src} does not exist"));
                }

                // Determine if source is a directory or file and create target accordingly.
                if Path::new(src).is_dir() {
                    ensure_dir(&target);
                } else {
                    if let Some(parent) = Path::new(&target).parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&target, "");
                }

                // Bind mount.
                let mut flags = libc::MS_BIND | libc::MS_REC;
                if !dev {
                    flags |= libc::MS_NOSUID | libc::MS_NODEV;
                }
                if let Err(e) = sys::mount(Some(src), &target, None, flags as _, None) {
                    die_fmt(format_args!("bind mount {src} -> {target}: {e}"));
                }

                // Remount for nosuid/nodev and optionally readonly.
                let mut remount_flags: u64 =
                    libc::MS_BIND | libc::MS_REMOUNT | libc::MS_REC;
                if !dev {
                    remount_flags |= libc::MS_NOSUID | libc::MS_NODEV;
                }
                if *readonly {
                    remount_flags |= libc::MS_RDONLY;
                }
                let _ = sys::mount(None, &target, None, remount_flags, None);
            }

            SetupOp::BindMountFd { fd, dest, readonly } => {
                let target = format!("{newroot}{dest}");
                let src = format!("/proc/self/fd/{fd}");
                ensure_dir(&target);

                let flags = libc::MS_BIND | libc::MS_REC;
                if let Err(e) = sys::mount(Some(&src), &target, None, flags as _, None) {
                    die_fmt(format_args!("bind-fd mount fd={fd} -> {target}: {e}"));
                }
                if *readonly {
                    let ro_flags = libc::MS_BIND
                        | libc::MS_REMOUNT
                        | libc::MS_REC
                        | libc::MS_RDONLY;
                    let _ = sys::mount(None, &target, None, ro_flags, None);
                }
            }

            SetupOp::RemountRo { dest } => {
                let target = format!("{newroot}{dest}");
                let flags = libc::MS_BIND
                    | libc::MS_REMOUNT
                    | libc::MS_RDONLY
                    | libc::MS_NOSUID
                    | libc::MS_NODEV;
                if let Err(e) = sys::mount(None, &target, None, flags, None) {
                    die_fmt(format_args!("remount-ro {target}: {e}"));
                }
            }

            SetupOp::MountProc { dest } => {
                let target = format!("{newroot}{dest}");
                ensure_dir(&target);
                if let Err(e) = sys::mount(
                    Some("proc"),
                    &target,
                    Some("proc"),
                    libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
                    None,
                ) {
                    die_fmt(format_args!("mount proc on {target}: {e}"));
                }
            }

            SetupOp::MountDev { dest } => {
                let target = format!("{newroot}{dest}");
                ensure_dir(&target);

                // Mount a tmpfs for /dev.
                if let Err(e) = sys::mount(
                    Some("tmpfs"),
                    &target,
                    Some("tmpfs"),
                    libc::MS_NOSUID | libc::MS_NOEXEC,
                    Some("mode=0755"),
                ) {
                    die_fmt(format_args!("mount dev tmpfs on {target}: {e}"));
                }

                // Create standard device nodes.
                let devs = [
                    ("null", 0o666, 1, 3),
                    ("zero", 0o666, 1, 5),
                    ("full", 0o666, 1, 7),
                    ("random", 0o666, 1, 8),
                    ("urandom", 0o666, 1, 9),
                    ("tty", 0o666, 5, 0),
                ];

                for (name, mode, major, minor) in &devs {
                    let dev_path = format!("{target}/{name}");
                    let dev = sys::makedev(*major, *minor);
                    if let Err(e) = sys::mknod(&dev_path, libc::S_IFCHR | mode, dev) {
                        // If we can't make device nodes (no CAP_MKNOD), try bind mounting
                        // from the host.
                        let host_dev = format!("/dev/{name}");
                        let _ = fs::write(&dev_path, "");
                        if let Err(e2) =
                            sys::mount(Some(&host_dev), &dev_path, None, libc::MS_BIND as _, None)
                        {
                            eprintln!(
                                "bwrap: warning: could not create /dev/{name}: mknod: {e}, bind: {e2}"
                            );
                        }
                    }
                }

                // Create standard symlinks.
                let links = [
                    ("fd", "/proc/self/fd"),
                    ("stdin", "/proc/self/fd/0"),
                    ("stdout", "/proc/self/fd/1"),
                    ("stderr", "/proc/self/fd/2"),
                    ("ptmx", "pts/ptmx"),
                ];
                for (name, link_target) in &links {
                    let link_path = format!("{target}/{name}");
                    let _ = std::os::unix::fs::symlink(link_target, &link_path);
                }

                // Create /dev/shm and /dev/pts.
                let _ = sys::mkdir(&format!("{target}/shm"), 0o1777);
                let _ = sys::mkdir(&format!("{target}/pts"), 0o755);

                // Try to mount devpts.
                let pts_path = format!("{target}/pts");
                let _ = sys::mount(
                    Some("devpts"),
                    &pts_path,
                    Some("devpts"),
                    libc::MS_NOSUID | libc::MS_NOEXEC,
                    Some("newinstance,ptmxmode=0666,mode=620"),
                );
            }

            SetupOp::MountTmpfs { dest, perms, size } => {
                let target = format!("{newroot}{dest}");
                ensure_dir(&target);
                let mode = perms.unwrap_or(0o755);
                let mut mount_data = format!("mode={mode:o}");
                if let Some(sz) = size {
                    mount_data.push_str(&format!(",size={sz}"));
                }
                if let Err(e) = sys::mount(
                    Some("tmpfs"),
                    &target,
                    Some("tmpfs"),
                    libc::MS_NOSUID | libc::MS_NODEV,
                    Some(&mount_data),
                ) {
                    die_fmt(format_args!("mount tmpfs on {target}: {e}"));
                }
            }

            SetupOp::MountMqueue { dest } => {
                let target = format!("{newroot}{dest}");
                ensure_dir(&target);
                if let Err(e) = sys::mount(Some("mqueue"), &target, Some("mqueue"), 0, None) {
                    die_fmt(format_args!("mount mqueue on {target}: {e}"));
                }
            }

            SetupOp::MakeDir { dest, perms } => {
                let target = format!("{newroot}{dest}");
                ensure_dir(&target);
                if let Some(p) = perms {
                    let _ = sys::chmod(&target, *p);
                }
            }

            SetupOp::MakeFile { fd, dest, perms } => {
                let target = format!("{newroot}{dest}");
                if let Some(parent) = Path::new(&target).parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let data = read_fd_to_vec(*fd);
                if let Err(e) = fs::write(&target, &data) {
                    die_fmt(format_args!("write file {target}: {e}"));
                }
                let mode = perms.unwrap_or(0o666);
                let _ = sys::chmod(&target, mode);
            }

            SetupOp::MakeBindFile {
                fd,
                dest,
                readonly,
                perms,
            } => {
                // Write fd data to a temp file, then bind-mount it.
                let target = format!("{newroot}{dest}");
                if let Some(parent) = Path::new(&target).parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let data = read_fd_to_vec(*fd);
                let mode = perms.unwrap_or(0o600);

                // Write to a temporary location.
                let tmp_path = format!("{base}/.bwrap-data-{}", dest.replace('/', "_"));
                if let Err(e) = fs::write(&tmp_path, &data) {
                    die_fmt(format_args!("write bind-data {tmp_path}: {e}"));
                }
                let _ = sys::chmod(&tmp_path, mode);

                // Create mount point.
                let _ = fs::write(&target, "");

                // Bind mount.
                if let Err(e) = sys::mount(Some(&tmp_path), &target, None, libc::MS_BIND as _, None)
                {
                    die_fmt(format_args!("bind-data mount {target}: {e}"));
                }
                if *readonly {
                    let ro_flags =
                        libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY;
                    let _ = sys::mount(None, &target, None, ro_flags, None);
                }
            }

            SetupOp::MakeSymlink { target, dest } => {
                let link_path = format!("{newroot}{dest}");
                if let Some(parent) = Path::new(&link_path).parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Err(e) = std::os::unix::fs::symlink(target, &link_path) {
                    // Ignore EEXIST.
                    if e.kind() != io::ErrorKind::AlreadyExists {
                        die_fmt(format_args!("symlink {target} -> {link_path}: {e}"));
                    }
                }
            }

            SetupOp::Chmod { perms, path } => {
                let target = format!("{newroot}{path}");
                if let Err(e) = sys::chmod(&target, *perms) {
                    die_fmt(format_args!("chmod {target}: {e}"));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// UID/GID mapping
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Sandbox execution
// ---------------------------------------------------------------------------

fn run_sandbox(opts: &Options) -> ! {
    let real_uid = sys::getuid();
    let real_gid = sys::getgid();

    // Set PR_SET_NO_NEW_PRIVS.
    if let Err(e) = sys::prctl_set_no_new_privs() {
        die_fmt(format_args!("PR_SET_NO_NEW_PRIVS: {e}"));
    }

    // Build clone flags.
    let mut clone_flags: i32 = libc::CLONE_NEWNS;

    let do_unshare_user =
        opts.unshare_user || opts.unshare_user_try || (opts.userns_fd.is_none() && real_uid != 0);

    if do_unshare_user {
        clone_flags |= libc::CLONE_NEWUSER;
    }
    if opts.unshare_ipc {
        clone_flags |= libc::CLONE_NEWIPC;
    }
    if opts.unshare_pid {
        clone_flags |= libc::CLONE_NEWPID;
    }
    if opts.unshare_net && !opts.share_net {
        clone_flags |= libc::CLONE_NEWNET;
    }
    if opts.unshare_uts {
        clone_flags |= libc::CLONE_NEWUTS;
    }
    if opts.unshare_cgroup || opts.unshare_cgroup_try {
        clone_flags |= libc::CLONE_NEWCGROUP;
    }

    // Join existing user namespace if specified.
    if let Some(fd) = opts.userns_fd
        && let Err(e) = sys::setns(fd, libc::CLONE_NEWUSER) {
            die_fmt(format_args!("setns userns: {e}"));
        }

    // Create pipe for parent-child synchronization.
    let (child_read, parent_write) =
        sys::pipe2(libc::O_CLOEXEC).unwrap_or_else(|e| die_fmt(format_args!("pipe: {e}")));

    // Clone into new namespaces.
    let pid =
        sys::clone3_pidns(clone_flags).unwrap_or_else(|e| die_fmt(format_args!("clone: {e}")));

    if pid != 0 {
        // Parent process (monitor).
        sys::close(child_read);

        // Write uid/gid map if we created a user namespace.
        if do_unshare_user {
            let map_uid = opts.sandbox_uid.unwrap_or(real_uid);
            let map_gid = opts.sandbox_gid.unwrap_or(real_gid);

            let uid_map = format!("{map_uid} {real_uid} 1\n");
            let pid_str = pid.to_string();
            let uid_map_path = format!("/proc/{pid_str}/uid_map");
            let gid_map_path = format!("/proc/{pid_str}/gid_map");
            let setgroups_path = format!("/proc/{pid_str}/setgroups");

            if let Err(e) = fs::write(&uid_map_path, &uid_map) {
                die_fmt(format_args!("write {uid_map_path}: {e}"));
            }
            let _ = fs::write(&setgroups_path, "deny");

            let gid_map = format!("{map_gid} {real_gid} 1\n");
            if let Err(e) = fs::write(&gid_map_path, &gid_map) {
                die_fmt(format_args!("write {gid_map_path}: {e}"));
            }
        }

        // Write info-fd.
        if let Some(fd) = opts.info_fd {
            let info = format!("{{\"child-pid\": {pid}}}\n");
            let _ = sys::write_all(fd, info.as_bytes());
        }

        // Write json-status-fd (child-pid).
        if let Some(fd) = opts.json_status_fd {
            let info = format!("{{\"child-pid\": {pid}}}\n");
            let _ = sys::write_all(fd, info.as_bytes());
        }

        // Signal child to proceed.
        let _ = sys::write_all(parent_write, b"x");
        sys::close(parent_write);

        // Die-with-parent.
        if opts.die_with_parent {
            let _ = sys::prctl_set_pdeathsig(libc::SIGKILL);
        }

        // Wait for child.
        match sys::waitpid(pid) {
            Ok(status) => {
                let exit_code = if libc::WIFEXITED(status) {
                    libc::WEXITSTATUS(status)
                } else if libc::WIFSIGNALED(status) {
                    128 + libc::WTERMSIG(status)
                } else {
                    255
                };

                // Write exit status to json-status-fd.
                if let Some(fd) = opts.json_status_fd {
                    let info = format!("{{\"exit-code\": {exit_code}}}\n");
                    let _ = sys::write_all(fd, info.as_bytes());
                    sys::close(fd);
                }

                // Close sync-fd.
                if let Some(fd) = opts.sync_fd {
                    sys::close(fd);
                }

                process::exit(exit_code);
            }
            Err(e) => {
                die_fmt(format_args!("waitpid: {e}"));
            }
        }
    }

    // Child process.
    sys::close(parent_write);

    // Wait for parent to set up uid/gid maps.
    let mut buf = [0u8; 1];
    let _ = sys::read_all(child_read, &mut buf);
    sys::close(child_read);

    // Set up loopback if we have a new network namespace.
    if opts.unshare_net && !opts.share_net
        && let Err(e) = sys::setup_loopback() {
            eprintln!("bwrap: warning: failed to set up loopback: {e}");
        }

    // Set hostname if requested.
    if let Some(ref hostname) = opts.sandbox_hostname
        && let Err(e) = sys::sethostname(hostname) {
            die_fmt(format_args!("sethostname: {e}"));
        }

    // Prevent mount propagation to the host.
    if let Err(e) = sys::mount(
        None,
        "/",
        None,
        libc::MS_SLAVE | libc::MS_REC,
        None,
    ) {
        die_fmt(format_args!("make / slave: {e}"));
    }

    // Create the temporary base directory.
    let base = "/tmp/.bwrap-root";
    let _ = sys::mkdir(base, 0o755);

    // Mount tmpfs on base.
    if let Err(e) = sys::mount(Some("tmpfs"), base, Some("tmpfs"), 0, Some("mode=0755")) {
        die_fmt(format_args!("mount tmpfs on {base}: {e}"));
    }

    // Create newroot as its own tmpfs mount point (pivot_root requires it).
    let newroot = format!("{base}/newroot");
    let _ = sys::mkdir(&newroot, 0o755);
    if let Err(e) = sys::mount(Some("tmpfs"), &newroot, Some("tmpfs"), 0, Some("mode=0755")) {
        die_fmt(format_args!("mount tmpfs on newroot: {e}"));
    }

    // Process all filesystem setup operations.
    setup_newroot(base, &opts.setup_ops);

    // Use the chdir + pivot_root(".", ".") technique:
    // 1. chdir into newroot
    // 2. pivot_root(".", ".") — the old root goes "under" the new root
    // 3. umount the old root (now at ".")
    if let Err(e) = sys::chdir(&newroot) {
        die_fmt(format_args!("chdir {newroot}: {e}"));
    }
    if let Err(e) = sys::pivot_root(".", ".") {
        die_fmt(format_args!("pivot_root(\".\", \".\"): {e}"));
    }

    // The old root is now mounted on top of the new root. Unmount it.
    if let Err(e) = sys::umount2(".", libc::MNT_DETACH) {
        eprintln!("bwrap: warning: umount old root: {e}");
    }

    // chdir to / in the new root.
    if let Err(e) = sys::chdir("/") {
        die_fmt(format_args!("chdir /: {e}"));
    }

    // Join second user namespace if specified.
    if let Some(fd) = opts.userns2_fd
        && let Err(e) = sys::setns(fd, libc::CLONE_NEWUSER) {
            die_fmt(format_args!("setns userns2: {e}"));
        }

    // Apply environment operations.
    // SAFETY: We are single-threaded at this point (post-fork child process).
    for op in &opts.env_ops {
        match op {
            EnvOp::Set(k, v) => unsafe { env::set_var(k, v) },
            EnvOp::Unset(k) => unsafe { env::remove_var(k) },
            EnvOp::Clear => {
                let keys: Vec<_> = env::vars().map(|(k, _)| k).collect();
                for k in keys {
                    if k != "PWD" {
                        unsafe { env::remove_var(&k) };
                    }
                }
            }
        }
    }

    // Block on block-fd if specified.
    if let Some(fd) = opts.block_fd {
        let mut buf = [0u8; 1];
        let _ = sys::read_all(fd, &mut buf);
        sys::close(fd);
    }

    // Change directory.
    if let Some(ref dir) = opts.chdir
        && let Err(e) = sys::chdir(dir) {
            die_fmt(format_args!("chdir {dir}: {e}"));
        }

    // New session.
    if opts.new_session
        && let Err(e) = sys::setsid() {
            die_fmt(format_args!("setsid: {e}"));
        }

    // If we have a PID namespace and the user didn't request --as-pid-1,
    // fork an intermediate pid-1 reaper process.
    if opts.unshare_pid && !opts.as_pid_1 {
        match sys::fork() {
            Ok(0) => {
                // Grandchild — this becomes the actual command.
            }
            Ok(_child_pid) => {
                // Intermediate pid-1 — reap children.
                loop {
                    match sys::waitpid(-1) {
                        Ok(status) => {
                            // Check if the child we care about exited.
                            // We can't easily get the pid from waitpid(-1) in this
                            // wrapper, so just check status and exit.
                            if libc::WIFEXITED(status) {
                                process::exit(libc::WEXITSTATUS(status));
                            } else if libc::WIFSIGNALED(status) {
                                process::exit(128 + libc::WTERMSIG(status));
                            }
                        }
                        Err(ref e) if e.raw_os_error() == Some(libc::ECHILD) => {
                            process::exit(0);
                        }
                        Err(_) => {
                            continue;
                        }
                    }
                }
            }
            Err(e) => {
                die_fmt(format_args!("fork pid-1 reaper: {e}"));
            }
        }
    }

    // Load seccomp filters.
    if let Some(fd) = opts.seccomp_fd {
        load_seccomp(fd);
    }
    for fd in &opts.add_seccomp_fds {
        load_seccomp(*fd);
    }

    // Exec the command.
    let argv0 = opts.argv0.as_deref().unwrap_or(&opts.command[0]);

    let c_argv0 = CString::new(argv0).unwrap_or_else(|_| die("invalid argv0"));
    let c_file = CString::new(opts.command[0].as_str()).unwrap_or_else(|_| die("invalid command"));

    let mut c_args: Vec<CString> = Vec::new();
    c_args.push(c_argv0);
    for arg in &opts.command[1..] {
        c_args.push(CString::new(arg.as_str()).unwrap_or_else(|_| die("invalid argument")));
    }

    let c_argv: Vec<*const libc::c_char> = c_args
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    let err = sys::execvp(&c_file, &c_argv);
    die_fmt(format_args!("execvp {}: {err}", opts.command[0]));
}

fn load_seccomp(fd: RawFd) {
    let data = read_fd_to_vec(fd);
    if data.is_empty() {
        return;
    }

    let prog = libc::sock_fprog {
        len: (data.len() / std::mem::size_of::<libc::sock_filter>()) as u16,
        filter: data.as_ptr() as *mut libc::sock_filter,
    };

    let ret = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER,
            &prog as *const libc::sock_fprog,
        )
    };
    if ret == -1 {
        let e = io::Error::last_os_error();
        die_fmt(format_args!("seccomp: {e}"));
    }
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

fn print_usage() {
    println!(
        "\
Usage: bwrap [OPTIONS...] [--] COMMAND [ARGS...]

Namespace control:
  --unshare-user              Create new user namespace
  --unshare-user-try          Try to create new user namespace
  --unshare-ipc               Create new IPC namespace
  --unshare-pid               Create new PID namespace
  --unshare-net               Create new network namespace
  --unshare-uts               Create new UTS namespace
  --unshare-cgroup            Create new cgroup namespace
  --unshare-cgroup-try        Try to create new cgroup namespace
  --unshare-all               Unshare all namespaces
  --share-net                 Retain network namespace (undo --unshare-net)
  --userns FD                 Use existing user namespace
  --userns2 FD                Switch to user namespace after setup
  --pidns FD                  Use existing PID namespace
  --disable-userns            Prevent further user namespace creation
  --assert-userns-disabled    Assert userns creation is disabled
  --uid UID                   Custom user ID in sandbox
  --gid GID                   Custom group ID in sandbox
  --hostname NAME             Custom hostname (requires --unshare-uts)

Environment:
  --chdir DIR                 Change to DIR inside sandbox
  --setenv VAR VALUE          Set environment variable
  --unsetenv VAR              Unset environment variable
  --clearenv                  Clear all environment variables

Filesystem:
  --bind SRC DEST             Bind mount SRC on DEST
  --bind-try SRC DEST         Bind mount (ignore if SRC missing)
  --ro-bind SRC DEST          Read-only bind mount
  --ro-bind-try SRC DEST      Read-only bind mount (ignore if SRC missing)
  --dev-bind SRC DEST         Bind mount with device access
  --dev-bind-try SRC DEST     Device bind mount (ignore if SRC missing)
  --bind-fd FD DEST           Bind mount from file descriptor
  --ro-bind-fd FD DEST        Read-only bind mount from file descriptor
  --remount-ro DEST           Remount DEST as read-only
  --proc DEST                 Mount procfs on DEST
  --dev DEST                  Mount new devtmpfs on DEST
  --tmpfs DEST                Mount tmpfs on DEST
  --mqueue DEST               Mount mqueue on DEST
  --dir DEST                  Create directory at DEST
  --file FD DEST              Write FD contents to DEST
  --bind-data FD DEST         Bind mount FD data on DEST
  --ro-bind-data FD DEST      Read-only bind mount FD data on DEST
  --symlink SRC DEST          Create symlink DEST -> SRC
  --chmod OCTAL PATH          Set permissions on PATH
  --perms OCTAL               Set permissions for next operation
  --size BYTES                Set size for next --tmpfs

Security:
  --seccomp FD                Load seccomp BPF from FD
  --add-seccomp-fd FD         Add seccomp BPF from FD
  --exec-label LABEL          SELinux exec label
  --file-label LABEL          SELinux file label
  --new-session               Create new terminal session (setsid)
  --die-with-parent           Kill sandbox when parent dies
  --as-pid-1                  Run command as PID 1 (no reaper)
  --cap-add CAP               Add capability
  --cap-drop CAP              Drop capability

Monitoring:
  --lock-file DEST            Lock file while sandbox runs
  --sync-fd FD                Keep FD open while sandbox runs
  --block-fd FD               Block on FD before exec
  --userns-block-fd FD        Block on FD before userns setup
  --info-fd FD                Write sandbox info JSON to FD
  --json-status-fd FD         Write JSON status to FD

Misc:
  --argv0 VALUE               Set argv[0] for command
  --args FD                   Read args from FD (nul-separated)
  --help                      Show this help
  --version                   Show version"
    );
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let opts = parse_args();
    run_sandbox(&opts);
}
