//! Sandbox construction using bwrap.
//!
//! Builds the bwrap command line for running a Flatpak application, including
//! all namespace setup, filesystem mounts, environment variables, device access,
//! and permission enforcement.

use std::env;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::process::Command;

use crate::dbus_proxy::{self, RunningProxy};
use crate::extensions;
use crate::installation::{DeployedRef, Installation};
use crate::metadata::ContextPermissions;
use crate::seccomp;

/// Builder for constructing bwrap arguments.
#[derive(Debug)]
pub struct BwrapBuilder {
    args: Vec<String>,
    env_vars: Vec<(String, String)>,
    unset_vars: Vec<String>,
    #[allow(dead_code)]
    fds: Vec<(i32, Vec<u8>)>,
}

impl BwrapBuilder {
    pub fn new() -> Self {
        Self {
            args: Vec::new(),
            env_vars: Vec::new(),
            unset_vars: Vec::new(),
            fds: Vec::new(),
        }
    }

    pub fn arg(&mut self, a: &str) -> &mut Self {
        self.args.push(a.to_string());
        self
    }

    pub fn args(&mut self, a: &[&str]) -> &mut Self {
        for s in a {
            self.args.push(s.to_string());
        }
        self
    }

    pub fn setenv(&mut self, key: &str, val: &str) -> &mut Self {
        self.env_vars.push((key.to_string(), val.to_string()));
        self
    }

    pub fn unsetenv(&mut self, key: &str) -> &mut Self {
        self.unset_vars.push(key.to_string());
        self
    }

    /// Build the final bwrap Command.
    pub fn build(&self, bwrap_path: &str, command: &[String]) -> Command {
        let mut cmd = Command::new(bwrap_path);

        for arg in &self.args {
            cmd.arg(arg);
        }

        for (k, v) in &self.env_vars {
            cmd.args(["--setenv", k, v]);
        }
        for k in &self.unset_vars {
            cmd.args(["--unsetenv", k]);
        }

        cmd.arg("--");
        for a in command {
            cmd.arg(a);
        }

        cmd
    }
}

/// Find the bwrap binary.
pub fn find_bwrap() -> String {
    // Check PATH for bwrap.
    if let Ok(path) = which("bwrap") {
        return path;
    }
    // Fallback.
    "bwrap".to_string()
}

fn which(name: &str) -> Result<String, ()> {
    if let Ok(path) = env::var("PATH") {
        for dir in path.split(':') {
            let candidate = format!("{dir}/{name}");
            if Path::new(&candidate).exists() {
                return Ok(candidate);
            }
        }
    }
    Err(())
}

/// A capability operation (add or drop).
pub enum CapOp {
    Add(String),
    Drop(String),
}

/// Result of building a sandbox: the bwrap command plus any proxy processes
/// that must stay alive during sandbox execution.
pub struct SandboxSetup {
    pub command: Command,
    pub _proxies: Vec<RunningProxy>,
    /// Read end of the --info-fd pipe (write end passed to bwrap).
    pub info_pipe_read: Option<RawFd>,
}

/// Build the complete bwrap command for running a Flatpak app.
#[allow(clippy::too_many_arguments)]
pub fn build_sandbox(
    deployed: &DeployedRef,
    runtime_deployed: Option<&DeployedRef>,
    extra_args: &[String],
    command_override: Option<&str>,
    devel: bool,
    sandbox_mode: bool,
    cap_ops: &[CapOp],
    instance_id: &str,
) -> Result<SandboxSetup, String> {
    let mut bwrap = BwrapBuilder::new();
    let app_id = &deployed.ref_.id;
    let metadata = &deployed.metadata;
    let ctx = metadata.context();

    // Load and merge overrides.
    let mut permissions = ctx.clone();
    for inst in Installation::all() {
        if let Some(overrides) = inst.load_overrides(app_id) {
            let override_ctx = overrides.context();
            permissions.merge(&override_ctx);
        }
    }

    // Reset permissions if sandbox mode.
    if sandbox_mode {
        permissions = ContextPermissions::default();
    }

    // --- Namespace setup ---
    bwrap.arg("--unshare-user");
    bwrap.arg("--disable-userns");
    bwrap.arg("--unshare-pid");
    bwrap.arg("--die-with-parent");
    bwrap.arg("--new-session"); // Prevent TIOCSTI terminal injection.

    if !permissions.has_shared("network") {
        bwrap.arg("--unshare-net");
    }
    if !permissions.has_shared("ipc") {
        bwrap.arg("--unshare-ipc");
    }
    bwrap.arg("--unshare-uts");
    bwrap.arg("--unshare-cgroup-try");

    // --- Mount runtime as /usr ---
    if let Some(rt) = runtime_deployed {
        let rt_files = rt.installation.files_path(&rt.ref_);
        if rt_files.exists() {
            bwrap.args(&["--ro-bind", &rt_files.to_string_lossy(), "/usr"]);
        }
    }

    // --- Mount app as /app ---
    let app_files = deployed.installation.files_path(&deployed.ref_);
    if app_files.exists() {
        bwrap.args(&["--ro-bind", &app_files.to_string_lossy(), "/app"]);
    }

    // --- Mount extensions ---
    if let Some(rt) = runtime_deployed {
        let rt_ref = &rt.ref_;
        let resolved = extensions::resolve_extensions(
            &rt.metadata,
            Some(&deployed.metadata),
            &Installation::all(),
            rt_ref,
        );

        if !resolved.is_empty() {
            let (ext_args, ld_paths) = extensions::extension_mount_args(&resolved, false);
            for arg in &ext_args {
                bwrap.arg(arg);
            }
            // Extend LD_LIBRARY_PATH with extension library paths.
            if !ld_paths.is_empty() {
                let ld_path = ld_paths.join(":");
                bwrap.setenv("LD_LIBRARY_PATH", &ld_path);

                // Regenerate ld.so.cache if possible.
                let rt_files = rt.installation.files_path(rt_ref);
                if let Some(cache_path) = extensions::regenerate_ld_cache(&rt_files, &ld_paths) {
                    bwrap.args(&[
                        "--ro-bind",
                        &cache_path.to_string_lossy(),
                        "/etc/ld.so.cache",
                    ]);
                }
            }
        }
    }

    // --- /proc, /dev, /tmp ---
    bwrap.arg("--proc");
    bwrap.arg("/proc");

    setup_devices(&mut bwrap, &permissions);

    // --- Per-app shared /tmp and /dev/shm ---
    let app_tmp = Installation::ensure_app_data_dirs(app_id)
        .parent()
        .unwrap()
        .join(".tmp");
    let _ = std::fs::create_dir_all(&app_tmp);
    bwrap.args(&["--bind", &app_tmp.to_string_lossy(), "/tmp"]);
    bwrap.args(&["--dir", "/var/tmp"]);

    // --- /sys (read-only) ---
    for sys_dir in &["block", "bus", "class", "dev", "devices"] {
        let path = format!("/sys/{sys_dir}");
        if Path::new(&path).exists() {
            bwrap.args(&["--ro-bind", &path, &path]);
        }
    }

    // --- Timezone ---
    setup_timezone(&mut bwrap);

    // --- Host fonts ---
    for font_dir in &["/usr/share/fonts", "/usr/local/share/fonts", "/etc/fonts"] {
        if Path::new(font_dir).exists() {
            let dest = format!("/run/host{font_dir}");
            bwrap.args(&["--ro-bind", font_dir, &dest]);
        }
    }
    let home = env::var("HOME").unwrap_or_else(|_| "/home/user".into());
    let user_fonts = format!("{home}/.local/share/fonts");
    if Path::new(&user_fonts).exists() {
        bwrap.args(&["--ro-bind", &user_fonts, "/run/host/user-fonts"]);
    }

    // --- Host icons ---
    for icon_dir in &["/usr/share/icons", "/usr/share/pixmaps"] {
        if Path::new(icon_dir).exists() {
            let dest = format!("/run/host{icon_dir}");
            bwrap.args(&["--ro-bind", icon_dir, &dest]);
        }
    }

    // --- Usr-merged symlinks ---
    for name in &["bin", "sbin", "lib", "lib32", "lib64"] {
        // Create usr-merged symlinks.
        bwrap.args(&["--symlink", &format!("usr/{name}"), &format!("/{name}")]);
    }

    // --- /etc from runtime ---
    setup_etc(&mut bwrap, runtime_deployed);

    // --- App data directories ---
    let app_data = Installation::ensure_app_data_dirs(app_id);
    bwrap.args(&[
        "--bind",
        &app_data.join("cache").to_string_lossy(),
        "/var/cache",
    ]);
    bwrap.args(&[
        "--bind",
        &app_data.join("data").to_string_lossy(),
        "/var/data",
    ]);
    bwrap.args(&[
        "--bind",
        &app_data.join("config").to_string_lossy(),
        "/var/config",
    ]);
    bwrap.args(&[
        "--bind",
        &app_data.join("cache/tmp").to_string_lossy(),
        "/var/tmp",
    ]);

    // --- XDG runtime dir ---
    let uid = unsafe { libc::getuid() };
    let xdg_runtime = format!("/run/user/{uid}");
    bwrap.args(&["--perms", "0700", "--dir", &xdg_runtime]);
    bwrap.setenv("XDG_RUNTIME_DIR", &xdg_runtime);

    // Misc dirs.
    bwrap.args(&["--dir", "/run/host"]);
    bwrap.args(&["--symlink", "../run", "/var/run"]);

    // --- Filesystem exports ---
    setup_filesystem_exports(&mut bwrap, &permissions);

    // --- Socket access ---
    setup_sockets(&mut bwrap, &permissions);

    // --- Default environment ---
    bwrap.setenv("PATH", "/app/bin:/usr/bin");
    bwrap.setenv("XDG_CONFIG_DIRS", "/app/etc/xdg:/etc/xdg");
    bwrap.setenv("XDG_DATA_DIRS", "/app/share:/usr/share");
    bwrap.setenv("SHELL", "/bin/sh");
    bwrap.setenv("FLATPAK_ID", app_id);
    bwrap.setenv("container", "flatpak");

    // Set XDG base dirs.
    bwrap.setenv("XDG_CACHE_HOME", "/var/cache");
    bwrap.setenv("XDG_DATA_HOME", "/var/data");
    bwrap.setenv("XDG_CONFIG_HOME", "/var/config");
    bwrap.setenv("XDG_STATE_HOME", "/var/data/.local/state");

    // Unset potentially dangerous vars.
    for var in &[
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "LD_AUDIT",
        "PYTHONPATH",
        "PERLLIB",
        "PERL5LIB",
    ] {
        bwrap.unsetenv(var);
    }

    // Apply [Environment] from metadata.
    for (k, v) in metadata.environment() {
        bwrap.setenv(&k, &v);
    }

    // Unset vars from context.
    for var in &permissions.unset_environment {
        bwrap.unsetenv(var);
    }

    if devel {
        bwrap.setenv("G_MESSAGES_DEBUG", "all");
    }

    // --- Capabilities ---
    // By default Flatpak drops all capabilities. Apply cap_ops in order.
    if !cap_ops.is_empty() {
        // If there are any cap operations, start by dropping all, then apply.
        let mut has_drop_all = false;
        for op in cap_ops {
            match op {
                CapOp::Drop(cap) => {
                    if cap == "ALL" {
                        has_drop_all = true;
                    }
                    bwrap.args(&["--cap-drop", cap]);
                }
                CapOp::Add(cap) => {
                    bwrap.args(&["--cap-add", cap]);
                }
            }
        }
        if !has_drop_all {
            // Ensure ALL caps are dropped first (Flatpak default).
            // Insert --cap-drop ALL at the beginning of the cap sequence.
            // Since bwrap processes args in order, we prepend it.
            // Actually bwrap applies all cap-drops then all cap-adds, so
            // we just add it here.
            bwrap.args(&["--cap-drop", "ALL"]);
        }
    }

    // --- Seccomp filter ---
    let seccomp_opts = seccomp::SeccompOptions {
        devel,
        bluetooth: permissions.has_feature("bluetooth"),
        canbus: permissions.has_feature("canbus"),
    };
    match seccomp::write_filter_to_memfd(&seccomp_opts) {
        Ok(fd) => {
            bwrap.args(&["--seccomp", &fd.to_string()]);
        }
        Err(e) => {
            eprintln!("flatpak: warning: could not create seccomp filter: {e}");
        }
    }

    // --- D-Bus proxy ---
    let mut proxies: Vec<RunningProxy> = Vec::new();

    let has_session_socket = permissions.has_socket("session-bus");
    let has_system_socket = permissions.has_socket("system-bus");

    // Session bus.
    if has_session_socket {
        // Direct, unfiltered session bus access.
        if let Some(socket) = dbus_proxy::session_bus_socket_path() {
            let uid = unsafe { libc::getuid() };
            let dest = format!("/run/user/{uid}/bus");
            bwrap.args(&["--ro-bind", &socket.to_string_lossy(), &dest]);
            bwrap.setenv("DBUS_SESSION_BUS_ADDRESS", &format!("unix:path={dest}"));
        }
    } else {
        // Filtered session bus via proxy.
        let session_policies = metadata.session_bus_policy();
        let (filtering, policies) = dbus_proxy::build_session_policies(&session_policies, false);

        if filtering {
            match dbus_proxy::launch_session_proxy(app_id, &policies, instance_id) {
                Ok(proxy) => {
                    let uid = unsafe { libc::getuid() };
                    let dest = format!("/run/user/{uid}/bus");
                    bwrap.args(&["--ro-bind", &proxy.socket_path.to_string_lossy(), &dest]);
                    bwrap.setenv("DBUS_SESSION_BUS_ADDRESS", &format!("unix:path={dest}"));
                    proxies.push(proxy);
                }
                Err(e) => {
                    eprintln!("flatpak: warning: session bus proxy failed: {e}");
                }
            }
        }
    }

    // System bus.
    if has_system_socket {
        if let Some(socket) = dbus_proxy::system_bus_socket_path() {
            bwrap.args(&[
                "--ro-bind",
                &socket.to_string_lossy(),
                "/run/dbus/system_bus_socket",
            ]);
        }
    } else {
        let system_policies = metadata.system_bus_policy();
        let (filtering, policies) = dbus_proxy::build_system_policies(&system_policies, false);

        if filtering {
            match dbus_proxy::launch_system_proxy(app_id, &policies, instance_id) {
                Ok(proxy) => {
                    bwrap.args(&[
                        "--ro-bind",
                        &proxy.socket_path.to_string_lossy(),
                        "/run/dbus/system_bus_socket",
                    ]);
                    proxies.push(proxy);
                }
                Err(e) => {
                    eprintln!("flatpak: warning: system bus proxy failed: {e}");
                }
            }
        }
    }

    // Document portal.
    let doc_mount = crate::portals::documents_mount_path();
    if doc_mount.exists() {
        bwrap.args(&[
            "--bind",
            &doc_mount.to_string_lossy(),
            &doc_mount.to_string_lossy(),
        ]);

        // Try to discover the portal PID for FLATPAK_PORTAL_PID.
        if let Some(pid) = discover_portal_pid() {
            bwrap.setenv("FLATPAK_PORTAL_PID", &pid.to_string());
        }
    }

    // Accessibility bus.
    if dbus_proxy::a11y_bus_address().is_some() {
        let a11y_policy_map = metadata
            .groups
            .get("Accessibility Bus Policy")
            .cloned()
            .unwrap_or_default();
        let (_filtering, policies) = dbus_proxy::build_a11y_policies(&a11y_policy_map);

        match dbus_proxy::launch_a11y_proxy(app_id, &policies, instance_id) {
            Ok(proxy) => {
                let uid = unsafe { libc::getuid() };
                let dest = format!("/run/user/{uid}/at-spi-bus");
                bwrap.args(&["--ro-bind", &proxy.socket_path.to_string_lossy(), &dest]);
                bwrap.setenv("AT_SPI_BUS_ADDRESS", &format!("unix:path={dest}"));
                proxies.push(proxy);
            }
            Err(_) => {
                // Not fatal — a11y bus may not be available.
            }
        }
    }

    // --- .flatpak-info via memfd ---
    let info_content = build_flatpak_info(deployed, runtime_deployed);
    match write_memfd("flatpak-info", info_content.as_bytes()) {
        Ok(fd) => {
            bwrap.args(&["--ro-bind-data", &fd.to_string(), "/.flatpak-info"]);
        }
        Err(_) => {
            // Fallback to temp file.
            let info_path = format!("/tmp/.flatpak-info-{}", std::process::id());
            let _ = std::fs::write(&info_path, &info_content);
            bwrap.args(&["--ro-bind", &info_path, "/.flatpak-info"]);
        }
    }

    // --- /run/host/container-manager ---
    if let Ok(fd) = write_memfd("container-manager", b"flatpak\n") {
        bwrap.args(&[
            "--ro-bind-data",
            &fd.to_string(),
            "/run/host/container-manager",
        ]);
    }

    // --- Determine command ---
    let cmd_name = command_override
        .or_else(|| metadata.command())
        .ok_or_else(|| "no command specified in metadata".to_string())?;

    let mut command = vec![cmd_name.to_string()];
    command.extend_from_slice(extra_args);

    // --- Info pipe for capturing child PID ---
    let info_pipe_read = create_info_pipe(&mut bwrap);

    let bwrap_path = find_bwrap();
    Ok(SandboxSetup {
        command: bwrap.build(&bwrap_path, &command),
        _proxies: proxies,
        info_pipe_read,
    })
}

/// Create a pipe and add --info-fd to the bwrap args.
/// Returns the read end of the pipe.
fn create_info_pipe(bwrap: &mut BwrapBuilder) -> Option<RawFd> {
    let mut fds = [0i32; 2];
    let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), 0) }; // No CLOEXEC on write end.
    if ret < 0 {
        return None;
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    bwrap.args(&["--info-fd", &write_fd.to_string()]);
    Some(read_fd)
}

fn setup_devices(bwrap: &mut BwrapBuilder, ctx: &ContextPermissions) {
    if ctx.has_device("all") {
        bwrap.args(&["--dev-bind", "/dev", "/dev"]);
    } else {
        bwrap.args(&["--dev", "/dev"]);

        if ctx.has_device("dri") {
            bwrap.args(&["--dev-bind-try", "/dev/dri", "/dev/dri"]);
            // NVIDIA devices.
            for dev in &[
                "/dev/nvidiactl",
                "/dev/nvidia-modeset",
                "/dev/nvidia-uvm",
                "/dev/nvidia-uvm-tools",
            ] {
                bwrap.args(&["--dev-bind-try", dev, dev]);
            }
            for i in 0..20 {
                let dev = format!("/dev/nvidia{i}");
                bwrap.args(&["--dev-bind-try", &dev, &dev]);
            }
        }
        if ctx.has_device("kvm") {
            bwrap.args(&["--dev-bind-try", "/dev/kvm", "/dev/kvm"]);
        }
        if ctx.has_device("input") {
            bwrap.args(&["--dev-bind-try", "/dev/input", "/dev/input"]);
        }
        if ctx.has_device("usb") {
            bwrap.args(&["--dev-bind-try", "/dev/bus/usb", "/dev/bus/usb"]);
        }
        if ctx.has_device("shm") {
            bwrap.args(&["--bind", "/dev/shm", "/dev/shm"]);
        }
    }
}

fn setup_etc(bwrap: &mut BwrapBuilder, runtime: Option<&DeployedRef>) {
    // Bind common /etc files from the host.
    let host_etc_files = [
        "resolv.conf",
        "hosts",
        "host.conf",
        "gai.conf",
        "nsswitch.conf",
        "machine-id",
        "localtime",
        "timezone",
    ];
    for name in &host_etc_files {
        let host_path = format!("/etc/{name}");
        if Path::new(&host_path).exists() {
            bwrap.args(&["--ro-bind", &host_path, &host_path]);
        }
    }

    // Bind /etc files from the runtime.
    if let Some(rt) = runtime {
        let etc_dir = rt.installation.files_path(&rt.ref_).join("etc");
        if etc_dir.exists()
            && let Ok(entries) = std::fs::read_dir(&etc_dir)
        {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip files we already handle.
                if host_etc_files.contains(&name.as_str()) {
                    continue;
                }
                let src = entry.path();
                let dest = format!("/etc/{name}");
                bwrap.args(&["--ro-bind", &src.to_string_lossy(), &dest]);
            }
        }
    }

    // Generate minimal /etc/passwd and /etc/group.
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    let user = env::var("USER").unwrap_or_else(|_| "user".into());
    let home = env::var("HOME").unwrap_or_else(|_| "/home/user".into());

    let passwd = format!(
        "{user}:x:{uid}:{gid}::{home}:/bin/sh\nnfsnobody:x:65534:65534:Nobody:/:/sbin/nologin\n"
    );
    let group = format!("{user}:x:{gid}:\nnfsnobody:x:65534:\n");

    // Use memfd for /etc/passwd and /etc/group to avoid temp files.
    match write_memfd("passwd", passwd.as_bytes()) {
        Ok(fd) => bwrap.args(&["--ro-bind-data", &fd.to_string(), "/etc/passwd"]),
        Err(_) => {
            let path = format!("/tmp/.flatpak-passwd-{}", std::process::id());
            let _ = std::fs::write(&path, &passwd);
            bwrap.args(&["--ro-bind", &path, "/etc/passwd"])
        }
    };
    match write_memfd("group", group.as_bytes()) {
        Ok(fd) => bwrap.args(&["--ro-bind-data", &fd.to_string(), "/etc/group"]),
        Err(_) => {
            let path = format!("/tmp/.flatpak-group-{}", std::process::id());
            let _ = std::fs::write(&path, &group);
            bwrap.args(&["--ro-bind", &path, "/etc/group"])
        }
    };
}

fn setup_filesystem_exports(bwrap: &mut BwrapBuilder, ctx: &ContextPermissions) {
    for fs_spec in &ctx.filesystems {
        let (path, readonly) = if let Some(stripped) = fs_spec.strip_suffix(":ro") {
            (stripped, true)
        } else if let Some(stripped) = fs_spec.strip_suffix(":rw") {
            (stripped, false)
        } else if let Some(stripped) = fs_spec.strip_suffix(":create") {
            // Create the directory if it doesn't exist.
            let resolved = resolve_filesystem_path(stripped);
            let _ = std::fs::create_dir_all(&resolved);
            (stripped, false)
        } else {
            (fs_spec.as_str(), false)
        };

        let resolved = resolve_filesystem_path(path);

        if !Path::new(&resolved).exists() {
            continue;
        }

        if readonly {
            bwrap.args(&["--ro-bind", &resolved, &resolved]);
        } else {
            bwrap.args(&["--bind", &resolved, &resolved]);
        }
    }
}

fn resolve_filesystem_path(spec: &str) -> String {
    let home = env::var("HOME").unwrap_or_else(|_| "/home/user".into());
    match spec {
        "home" => home,
        "host" => "/".into(),
        "host-os" => "/usr".into(),
        "host-etc" => "/etc".into(),
        s if s.starts_with("~/") => format!("{home}/{}", &s[2..]),
        s if s.starts_with("xdg-desktop") => xdg_user_dir("DESKTOP", "Desktop"),
        s if s.starts_with("xdg-documents") => xdg_user_dir("DOCUMENTS", "Documents"),
        s if s.starts_with("xdg-download") => xdg_user_dir("DOWNLOAD", "Downloads"),
        s if s.starts_with("xdg-music") => xdg_user_dir("MUSIC", "Music"),
        s if s.starts_with("xdg-pictures") => xdg_user_dir("PICTURES", "Pictures"),
        s if s.starts_with("xdg-public-share") => xdg_user_dir("PUBLICSHARE", "Public"),
        s if s.starts_with("xdg-templates") => xdg_user_dir("TEMPLATES", "Templates"),
        s if s.starts_with("xdg-videos") => xdg_user_dir("VIDEOS", "Videos"),
        s if s.starts_with("xdg-config") => {
            env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"))
        }
        s if s.starts_with("xdg-data") => {
            env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{home}/.local/share"))
        }
        s if s.starts_with("xdg-cache") => {
            env::var("XDG_CACHE_HOME").unwrap_or_else(|_| format!("{home}/.cache"))
        }
        s if s.starts_with("xdg-run") => {
            let uid = unsafe { libc::getuid() };
            let base = format!("/run/user/{uid}");
            if s.len() > 8 {
                format!("{base}/{}", &s[8..])
            } else {
                base
            }
        }
        s => s.to_string(),
    }
}

fn xdg_user_dir(env_suffix: &str, default_name: &str) -> String {
    let home = env::var("HOME").unwrap_or_else(|_| "/home/user".into());
    env::var(format!("XDG_{env_suffix}_DIR")).unwrap_or_else(|_| format!("{home}/{default_name}"))
}

fn setup_sockets(bwrap: &mut BwrapBuilder, ctx: &ContextPermissions) {
    // Wayland.
    let want_wayland = ctx.has_socket("wayland") || ctx.has_socket("fallback-x11");
    let inherit_wayland = ctx.has_socket("inherit-wayland-socket");

    if (want_wayland || inherit_wayland)
        && let Ok(display) = env::var("WAYLAND_DISPLAY")
    {
        let uid = unsafe { libc::getuid() };
        let socket_path = format!("/run/user/{uid}/{display}");
        if Path::new(&socket_path).exists() {
            let dest = format!("/run/user/{uid}/{display}");
            bwrap.args(&["--ro-bind", &socket_path, &dest]);
            bwrap.setenv("WAYLAND_DISPLAY", &display);
        }
    }

    // X11.
    if (ctx.has_socket("x11") || ctx.has_socket("fallback-x11"))
        && let Ok(display) = env::var("DISPLAY")
    {
        bwrap.setenv("DISPLAY", &display);
        // Bind X11 socket.
        let x11_dir = "/tmp/.X11-unix";
        if Path::new(x11_dir).exists() {
            bwrap.args(&["--ro-bind", x11_dir, x11_dir]);
        }
        // Xauthority.
        if let Ok(xauth) = env::var("XAUTHORITY")
            && Path::new(&xauth).exists()
        {
            bwrap.args(&["--ro-bind", &xauth, &xauth]);
            bwrap.setenv("XAUTHORITY", &xauth);
        }
    }

    // PulseAudio.
    if ctx.has_socket("pulseaudio") {
        let uid = unsafe { libc::getuid() };
        let pulse_socket = format!("/run/user/{uid}/pulse/native");
        if Path::new(&pulse_socket).exists() {
            bwrap.args(&["--ro-bind", &pulse_socket, &pulse_socket]);
        }
        // PulseAudio config.
        let pulse_config = format!("/run/user/{uid}/pulse");
        if Path::new(&pulse_config).exists() {
            bwrap.args(&["--ro-bind", &pulse_config, &pulse_config]);
        }
    }

    // SSH auth.
    if ctx.has_socket("ssh-auth")
        && let Ok(sock) = env::var("SSH_AUTH_SOCK")
        && Path::new(&sock).exists()
    {
        let dest = "/run/flatpak/ssh-auth";
        bwrap.args(&["--ro-bind", &sock, dest]);
        bwrap.setenv("SSH_AUTH_SOCK", dest);
    }

    // CUPS.
    if ctx.has_socket("cups") {
        let cups_socket = "/run/cups/cups.sock";
        if Path::new(cups_socket).exists() {
            bwrap.args(&["--ro-bind", cups_socket, cups_socket]);
        }
    }
}

/// Write data to a memfd and return the file descriptor.
fn write_memfd(name: &str, data: &[u8]) -> Result<i32, String> {
    let c_name = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
    let fd = unsafe { libc::memfd_create(c_name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err("memfd_create failed".into());
    }
    let written = unsafe { libc::write(fd, data.as_ptr() as *const libc::c_void, data.len()) };
    if written < 0 || written as usize != data.len() {
        unsafe { libc::close(fd) };
        return Err("write to memfd failed".into());
    }
    unsafe { libc::lseek(fd, 0, libc::SEEK_SET) };
    Ok(fd)
}

/// Set up timezone in the sandbox.
/// Try to discover the document portal's PID.
fn discover_portal_pid() -> Option<u32> {
    // The portal runs as xdg-document-portal. Try to find its PID via pidof.
    let output = std::process::Command::new("pidof")
        .arg("xdg-document-portal")
        .output()
        .ok()?;
    if output.status.success() {
        let pid_str = String::from_utf8_lossy(&output.stdout);
        pid_str.split_whitespace().next()?.parse().ok()
    } else {
        None
    }
}

fn setup_timezone(bwrap: &mut BwrapBuilder) {
    // Bind the host zoneinfo database.
    if Path::new("/usr/share/zoneinfo").exists() {
        bwrap.args(&["--ro-bind", "/usr/share/zoneinfo", "/usr/share/zoneinfo"]);
    }

    // Determine the current timezone.
    let tz = std::fs::read_to_string("/etc/timezone")
        .ok()
        .map(|s| s.trim().to_string())
        .or_else(|| {
            // Try reading the /etc/localtime symlink target.
            std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                p.to_string_lossy()
                    .strip_prefix("/usr/share/zoneinfo/")
                    .map(String::from)
            })
        })
        .unwrap_or_else(|| "UTC".to_string());

    // Create /etc/localtime as a symlink to the zoneinfo file.
    let tz_path = format!("/usr/share/zoneinfo/{tz}");
    bwrap.args(&["--symlink", &tz_path, "/etc/localtime"]);

    // Write /etc/timezone.
    let tz_content = format!("{tz}\n");
    if let Ok(fd) = write_memfd("timezone", tz_content.as_bytes()) {
        bwrap.args(&["--ro-bind-data", &fd.to_string(), "/etc/timezone"]);
    }

    bwrap.setenv("TZ", &tz);
}

/// Build the .flatpak-info content for an app instance.
pub fn get_flatpak_info(deployed: &DeployedRef, runtime: Option<&DeployedRef>) -> String {
    build_flatpak_info(deployed, runtime)
}

fn build_flatpak_info(deployed: &DeployedRef, runtime: Option<&DeployedRef>) -> String {
    let mut info = String::from("[Application]\n");
    info.push_str(&format!("name={}\n", deployed.ref_.id));
    info.push_str(&format!(
        "runtime=runtime/{}\n",
        runtime.map(|r| r.ref_.to_string()).unwrap_or_default()
    ));

    info.push_str("\n[Instance]\n");
    info.push_str(&format!(
        "app-path={}\n",
        deployed.installation.files_path(&deployed.ref_).display()
    ));
    if let Some(rt) = runtime {
        info.push_str(&format!(
            "runtime-path={}\n",
            rt.installation.files_path(&rt.ref_).display()
        ));
    }
    info.push_str(&format!("arch={}\n", deployed.ref_.arch));
    info.push_str(&format!("branch={}\n", deployed.ref_.branch));
    info.push_str("flatpak-version=0.1.0 (rust-flatpak)\n");

    info
}
