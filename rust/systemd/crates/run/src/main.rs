//! systemd-run — Run a command as a transient service or scope unit.
//!
//! A drop-in replacement for `systemd-run(1)`. This tool creates a
//! transient `.service` or `.scope` unit and runs the specified command
//! within it.
//!
//! When full D-Bus/control-socket integration with the service manager is
//! not yet available, systemd-run falls back to direct execution with the
//! requested properties applied at the process level (user/group switching,
//! environment setup, etc.).
//!
//! Supported options:
//!
//! - `--scope`            — Run as a scope unit instead of a service unit
//! - `--unit=NAME`        — Use NAME as the transient unit name
//! - `--description=TEXT` — Set a description for the transient unit
//! - `--slice=SLICE`      — Place the unit in the specified slice
//! - `--property=PROP`    — Set a unit property (can be specified multiple times)
//! - `--service-type=TYPE`— Set the service type (simple, forking, oneshot, etc.)
//! - `--uid=USER`         — Run the command as the specified user
//! - `--gid=GROUP`        — Run the command with the specified group
//! - `-t`, `--pty`        — Request a pseudo-TTY for the command
//! - `--pipe`             — Use pipe for stdin/stdout/stderr
//! - `-S`, `--shell`      — Start a shell if no command is given
//! - `--wait`             — Wait for the unit to finish and show its result
//! - `--collect`          — Unload the unit after it finished (even if failed)
//! - `--working-directory`— Set the working directory for the command
//! - `-E`, `--setenv`     — Set environment variables for the command
//! - `--remain-after-exit`— Keep the unit around after the process exits
//! - `--send-sighup`      — Send SIGHUP to remaining processes after main exits
//! - `--no-block`         — Do not wait for the unit to start
//! - `--on-active=`       — Define a timer that activates after a delay
//! - `--on-boot=`         — Define a timer relative to boot
//! - `--on-calendar=`     — Define a calendar timer
//! - `--timer-property=`  — Set a property on the timer unit

use clap::Parser;
use std::ffi::CString;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "systemd-run",
    about = "Run programs in transient scope or service units",
    version,
    trailing_var_arg = true
)]
struct Cli {
    /// Run as a scope unit instead of a service unit.
    #[arg(long)]
    scope: bool,

    /// Use the specified unit name for the transient unit.
    #[arg(short = 'u', long, value_name = "NAME")]
    unit: Option<String>,

    /// Set a human-readable description for the unit. Can be specified multiple times; the last value wins.
    #[arg(long, value_name = "TEXT")]
    description: Vec<String>,

    /// Place the transient unit in the specified slice.
    #[arg(long, value_name = "SLICE")]
    slice: Option<String>,

    /// Set a unit property. Can be specified multiple times.
    /// Format: NAME=VALUE
    #[arg(short = 'p', long, value_name = "NAME=VALUE")]
    property: Vec<String>,

    /// Set the service type (simple, forking, oneshot, exec, notify, idle, dbus).
    /// Can be specified multiple times; the last value wins.
    #[arg(long, value_name = "TYPE")]
    service_type: Vec<String>,

    /// Run the command as the specified user.
    #[arg(long, value_name = "USER")]
    uid: Option<String>,

    /// Run the command with the specified group.
    #[arg(long, value_name = "GROUP")]
    gid: Option<String>,

    /// Request a pseudo-TTY for the command.
    #[arg(short = 't', long)]
    pty: bool,

    /// Use pipe for stdin/stdout/stderr (standard I/O forwarding).
    #[arg(short = 'P', long)]
    pipe: bool,

    /// Invoke a shell if no command is specified.
    #[arg(short = 'S', long)]
    shell: bool,

    /// Wait for the service to finish and show its result.
    #[arg(long)]
    wait: bool,

    /// Unload the transient unit after it finished, even if it failed.
    #[arg(long)]
    collect: bool,

    /// Set the working directory for the spawned process.
    #[arg(long, value_name = "DIR")]
    working_directory: Option<PathBuf>,

    /// Set an environment variable for the spawned process. Can be
    /// specified multiple times. Format: NAME=VALUE
    #[arg(short = 'E', long = "setenv", value_name = "NAME=VALUE")]
    setenv: Vec<String>,

    /// Keep the unit loaded after the main process exits.
    #[arg(short = 'r', long)]
    remain_after_exit: bool,

    /// Send SIGHUP to remaining processes when the main process exits.
    #[arg(long)]
    send_sighup: bool,

    /// Set the nice level of the spawned process.
    #[arg(long, value_name = "NICE", allow_hyphen_values = true)]
    nice: Option<i32>,

    /// Suppress informational messages, only show errors.
    #[arg(short = 'q', long)]
    quiet: bool,

    /// Do not query the user for authentication.
    #[arg(long)]
    no_ask_password: bool,

    /// Do not pipe output into a pager.
    #[arg(long)]
    no_pager: bool,

    /// Do not synchronously wait for the unit to start.
    #[arg(long)]
    no_block: bool,

    /// Define a relative timer: run the command after the specified delay.
    /// Accepts a time span like "5min" or "2h 30min". Can be specified multiple times.
    #[arg(long, value_name = "TIMESPAN")]
    on_active: Vec<String>,

    /// Define a timer relative to system boot.
    #[arg(long, value_name = "TIMESPAN")]
    on_boot: Option<String>,

    /// Define a timer relative to when the service manager was first started.
    #[arg(long, value_name = "TIMESPAN")]
    on_startup: Option<String>,

    /// Define a timer relative to when the unit was last activated.
    #[arg(long, value_name = "TIMESPAN")]
    on_unit_active: Option<String>,

    /// Define a timer relative to when the unit was last deactivated.
    #[arg(long, value_name = "TIMESPAN")]
    on_unit_inactive: Option<String>,

    /// Define a calendar-based timer (e.g. "Mon *-*-* 03:00:00").
    #[arg(long, value_name = "SPEC")]
    on_calendar: Option<String>,

    /// Run the command when the system clock (CLOCK_REALTIME) jumps
    /// relative to the monotonic clock.
    #[arg(long, action = clap::ArgAction::Count)]
    on_clock_change: u8,

    /// Run the command when the system timezone changes.
    #[arg(long, action = clap::ArgAction::Count)]
    on_timezone_change: u8,

    /// Set a property on the timer unit. Can be specified multiple times.
    #[arg(long, value_name = "NAME=VALUE")]
    timer_property: Vec<String>,

    /// Set a property on the path unit. Can be specified multiple times.
    #[arg(long, value_name = "NAME=VALUE")]
    path_property: Vec<String>,

    /// Set a property on the socket unit. Can be specified multiple times.
    #[arg(long, value_name = "NAME=VALUE")]
    socket_property: Vec<String>,

    /// Shortcut for --pipe --wait --service-type=exec. Shows command output.
    #[arg(short = 'v')]
    verbose: bool,

    /// Connect to the user service manager instead of the system one.
    #[arg(long)]
    user: bool,

    /// Connect to the system service manager (default).
    #[arg(long)]
    system: bool,

    /// Execute operation on a local container or on the host.
    /// Format: [user@]machine. Accepted for compatibility.
    #[arg(short = 'M', long, value_name = "MACHINE")]
    machine: Option<String>,

    /// The command and its arguments to run.
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

/// Generate a transient unit name from the command if --unit was not given.
fn generate_unit_name(command: &[String], scope: bool) -> String {
    let suffix = if scope { ".scope" } else { ".service" };
    // Use a unique identifier: PID + monotonic clock to avoid collisions
    // when multiple systemd-run invocations run concurrently.
    let unique = {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        }
        // Combine PID + nanoseconds for uniqueness
        format!(
            "{}{}",
            std::process::id(),
            (ts.tv_sec as u64)
                .wrapping_mul(1_000_000_000)
                .wrapping_add(ts.tv_nsec as u64)
        )
    };

    if command.is_empty() {
        return format!("run-u{unique}{suffix}");
    }

    // Use the basename of the command as the unit name
    let cmd = &command[0];
    let basename = std::path::Path::new(cmd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "run".to_string());

    // Sanitize: replace any character that's not alphanumeric, dash, or
    // underscore with an underscore.
    let sanitized: String = basename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    format!("run-u{unique}-{sanitized}{suffix}")
}

/// Look up a username and return (uid, gid, home, shell).
fn lookup_user(username: &str) -> Result<(u32, u32, String, String), String> {
    // Try numeric UID first
    if let Ok(uid) = username.parse::<u32>() {
        return Ok((uid, uid, "/".to_string(), "/bin/sh".to_string()));
    }

    let c_name = CString::new(username).map_err(|e| format!("Invalid username: {e}"))?;

    unsafe {
        let pwd = libc::getpwnam(c_name.as_ptr());
        if pwd.is_null() {
            return Err(format!("User not found: {username}"));
        }

        let uid = (*pwd).pw_uid;
        let gid = (*pwd).pw_gid;

        let home = if (*pwd).pw_dir.is_null() {
            "/".to_string()
        } else {
            std::ffi::CStr::from_ptr((*pwd).pw_dir)
                .to_string_lossy()
                .to_string()
        };

        let shell = if (*pwd).pw_shell.is_null() {
            "/bin/sh".to_string()
        } else {
            std::ffi::CStr::from_ptr((*pwd).pw_shell)
                .to_string_lossy()
                .to_string()
        };

        Ok((uid, gid, home, shell))
    }
}

/// Look up a group name and return its GID.
fn lookup_group(groupname: &str) -> Result<u32, String> {
    // Try numeric GID first
    if let Ok(gid) = groupname.parse::<u32>() {
        return Ok(gid);
    }

    let c_name = CString::new(groupname).map_err(|e| format!("Invalid group name: {e}"))?;

    unsafe {
        let grp = libc::getgrnam(c_name.as_ptr());
        if grp.is_null() {
            return Err(format!("Group not found: {groupname}"));
        }
        Ok((*grp).gr_gid)
    }
}

/// Determine the default shell for the current user.
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// Print information about the transient unit that would be created.
fn print_unit_info(cli: &Cli, unit_name: &str) {
    if cli.quiet {
        return;
    }
    eprintln!("Running as unit: {unit_name}");

    if let Some(desc) = cli.description.last() {
        eprintln!("Description: {desc}");
    }

    if let Some(ref slice) = cli.slice {
        eprintln!("Slice: {slice}");
    }

    if let Some(stype) = cli.service_type.last() {
        eprintln!("Service type: {stype}");
    }

    for prop in &cli.property {
        eprintln!("Property: {prop}");
    }

    if let Some(ref uid) = cli.uid {
        eprintln!("User: {uid}");
    }

    if let Some(ref gid) = cli.gid {
        eprintln!("Group: {gid}");
    }

    if let Some(ref wd) = cli.working_directory {
        eprintln!("Working directory: {}", wd.display());
    }

    for env in &cli.setenv {
        eprintln!("Environment: {env}");
    }

    if let Some(nice) = cli.nice {
        eprintln!("Nice: {nice}");
    }

    if cli.remain_after_exit {
        eprintln!("RemainAfterExit: yes");
    }

    if cli.scope {
        eprintln!("Mode: scope");
    }

    if cli.wait {
        eprintln!("Wait: yes");
    }

    for on_active in &cli.on_active {
        eprintln!("OnActiveSec: {on_active}");
    }

    if let Some(ref on_boot) = cli.on_boot {
        eprintln!("OnBootSec: {on_boot}");
    }

    if let Some(ref on_calendar) = cli.on_calendar {
        eprintln!("OnCalendar: {on_calendar}");
    }
}

/// Apply per-process properties before exec'ing the command.
///
/// This is the "fallback" execution mode that applies what we can at the
/// process level without needing to talk to the service manager via D-Bus.
fn apply_process_properties(cli: &Cli) -> Result<(), String> {
    // Apply GID first (needs to happen before dropping root)
    if let Some(ref gid_str) = cli.gid {
        let gid = lookup_group(gid_str)?;
        let ret = unsafe { libc::setgid(gid) };
        if ret != 0 {
            return Err(format!(
                "Failed to set GID to {gid}: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    // Apply UID
    if let Some(ref uid_str) = cli.uid {
        let (uid, default_gid, home, _shell) = lookup_user(uid_str)?;

        // If no explicit --gid was given, use the user's primary group
        if cli.gid.is_none() {
            let ret = unsafe { libc::setgid(default_gid) };
            if ret != 0 {
                return Err(format!(
                    "Failed to set GID to {default_gid}: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }

        let ret = unsafe { libc::setuid(uid) };
        if ret != 0 {
            return Err(format!(
                "Failed to set UID to {uid}: {}",
                std::io::Error::last_os_error()
            ));
        }

        // Update HOME environment variable
        unsafe { std::env::set_var("HOME", &home) };
    }

    // Apply nice level
    if let Some(nice) = cli.nice {
        let ret = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, nice) };
        if ret != 0 {
            return Err(format!(
                "Failed to set nice level to {nice}: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    // Apply working directory
    if let Some(ref wd) = cli.working_directory {
        std::env::set_current_dir(wd)
            .map_err(|e| format!("Failed to change directory to {}: {e}", wd.display()))?;
    }

    // Apply environment variables
    for env_spec in &cli.setenv {
        if let Some((key, value)) = env_spec.split_once('=') {
            unsafe { std::env::set_var(key, value) };
        } else {
            // If no '=' is present, interpret as just a key to pass through
            // from the current environment (matching systemd-run behavior)
            if let Ok(val) = std::env::var(env_spec) {
                unsafe { std::env::set_var(env_spec, &val) };
            }
        }
    }

    Ok(())
}

/// Try to connect to the rust-systemd control socket and create a
/// transient unit. Returns `Ok(Some(response))` if successful, `Ok(None)` if
/// the control socket is not available (falling back to direct exec).
fn try_create_transient_unit(
    cli: &Cli,
    unit_name: &str,
) -> Result<Option<serde_json::Value>, String> {
    use libsystemd::control::jsonrpc2::Call;
    use serde_json::Value;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let socket_path = "/run/systemd/rust-systemd-notify/control.socket";

    let stream = match UnixStream::connect(socket_path) {
        Ok(s) => s,
        Err(_) => return Ok(None), // Control socket not available
    };

    // Build the transient unit creation request.
    // The rust-systemd control protocol uses JSON-RPC 2.0.
    let mut properties = serde_json::Map::new();
    properties.insert("unit".into(), Value::String(unit_name.to_string()));

    if let Some(desc) = cli.description.last() {
        properties.insert("description".into(), Value::String(desc.clone()));
    }

    if !cli.command.is_empty() {
        let cmd_array: Vec<Value> = cli
            .command
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect();
        properties.insert("command".into(), Value::Array(cmd_array));
    }

    if let Some(ref uid) = cli.uid {
        properties.insert("user".into(), Value::String(uid.clone()));
    }

    if let Some(ref gid) = cli.gid {
        properties.insert("group".into(), Value::String(gid.clone()));
    }

    if let Some(nice) = cli.nice {
        properties.insert("nice".into(), Value::Number(nice.into()));
    }

    if let Some(ref wd) = cli.working_directory {
        properties.insert(
            "working_directory".into(),
            Value::String(wd.to_string_lossy().to_string()),
        );
    }

    if cli.scope {
        properties.insert("scope".into(), Value::Bool(true));
    }

    if cli.wait || cli.pipe {
        properties.insert("wait".into(), Value::Bool(true));
    }

    if cli.pipe {
        properties.insert("pipe".into(), Value::Bool(true));
    }

    if let Some(ref slice) = cli.slice {
        properties.insert("slice".into(), Value::String(slice.clone()));
    }

    if let Some(service_type) = cli.service_type.last() {
        properties.insert("service_type".into(), Value::String(service_type.clone()));
    }

    if cli.remain_after_exit {
        properties.insert("remain_after_exit".into(), Value::Bool(true));
    }

    // Pass -p / --property overrides
    if !cli.property.is_empty() {
        let props: Vec<Value> = cli
            .property
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect();
        properties.insert("properties".into(), Value::Array(props));
    }

    // Pass environment variables (resolve pass-through vars client-side)
    if !cli.setenv.is_empty() {
        let envs: Vec<Value> = cli
            .setenv
            .iter()
            .filter_map(|s| {
                if s.contains('=') {
                    Some(Value::String(s.clone()))
                } else {
                    // Pass-through: resolve from current environment
                    std::env::var(s)
                        .ok()
                        .map(|val| Value::String(format!("{s}={val}")))
                }
            })
            .collect();
        if !envs.is_empty() {
            properties.insert("environment".into(), Value::Array(envs));
        }
    }

    // Pass timer properties
    if let Some(ref on_calendar) = cli.on_calendar {
        properties.insert("on_calendar".into(), Value::String(on_calendar.clone()));
    }
    if !cli.on_active.is_empty() {
        let vals: Vec<Value> = cli
            .on_active
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect();
        properties.insert("on_active".into(), Value::Array(vals));
    }
    if let Some(ref on_boot) = cli.on_boot {
        properties.insert("on_boot".into(), Value::String(on_boot.clone()));
    }
    if let Some(ref on_startup) = cli.on_startup {
        properties.insert("on_startup".into(), Value::String(on_startup.clone()));
    }
    if let Some(ref on_unit_active) = cli.on_unit_active {
        properties.insert(
            "on_unit_active".into(),
            Value::String(on_unit_active.clone()),
        );
    }
    if let Some(ref on_unit_inactive) = cli.on_unit_inactive {
        properties.insert(
            "on_unit_inactive".into(),
            Value::String(on_unit_inactive.clone()),
        );
    }
    if cli.on_clock_change > 0 {
        properties.insert("on_clock_change".into(), Value::Bool(true));
    }
    if cli.on_timezone_change > 0 {
        properties.insert("on_timezone_change".into(), Value::Bool(true));
    }
    if !cli.timer_property.is_empty() {
        let tprops: Vec<Value> = cli
            .timer_property
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect();
        properties.insert("timer_properties".into(), Value::Array(tprops));
    }
    if !cli.path_property.is_empty() {
        let pprops: Vec<Value> = cli
            .path_property
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect();
        properties.insert("path_properties".into(), Value::Array(pprops));
    }
    if !cli.socket_property.is_empty() {
        let sprops: Vec<Value> = cli
            .socket_property
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect();
        properties.insert("socket_properties".into(), Value::Array(sprops));
    }

    let params = Value::Object(properties);

    let call = Call {
        method: "start-transient".to_string(),
        params: Some(params),
        id: None,
    };

    let payload = serde_json::to_string(&call.to_json())
        .map_err(|e| format!("Failed to serialize request: {e}"))?;

    let mut stream = stream;
    stream
        .write_all(payload.as_bytes())
        .map_err(|e| format!("Failed to send to control socket: {e}"))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("Failed to shutdown write end: {e}"))?;

    let resp: Value = serde_json::from_reader(&mut stream)
        .map_err(|e| format!("Failed to read response: {e}"))?;

    // Check for errors in the JSON-RPC response
    if let Some(error) = resp.get("error") {
        let msg = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(format!("Service manager error: {msg}"));
    }

    // Return the "result" field from the JSON-RPC response
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Ok(Some(result))
}

fn main() {
    // Ignore SIGPIPE so piping output to grep/head/etc. doesn't panic.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let mut cli = Cli::parse();

    // -v is shorthand for --pipe --wait --service-type=exec
    if cli.verbose {
        cli.pipe = true;
        cli.wait = true;
        if cli.service_type.is_empty() {
            cli.service_type.push("exec".to_string());
        }
    }

    // Determine the command to run
    let command = if cli.command.is_empty() && cli.shell {
        vec![default_shell()]
    } else {
        cli.command.clone()
    };

    // If a timer/path/socket is requested but no command, that's an error
    let has_timer = !cli.on_active.is_empty()
        || cli.on_boot.is_some()
        || cli.on_startup.is_some()
        || cli.on_unit_active.is_some()
        || cli.on_unit_inactive.is_some()
        || cli.on_calendar.is_some()
        || cli.on_clock_change > 0
        || cli.on_timezone_change > 0
        || !cli.timer_property.is_empty()
        || !cli.path_property.is_empty()
        || !cli.socket_property.is_empty();

    if command.is_empty() && !has_timer {
        eprintln!("Error: No command specified. Use --shell to start a shell.");
        process::exit(1);
    }

    // Reject empty command strings (e.g. `systemd-run ""`)
    if let Some(first) = command.first()
        && first.is_empty()
    {
        eprintln!("Error: Empty command specified.");
        process::exit(1);
    }

    // Generate or use the given unit name
    let mut unit_name = cli
        .unit
        .clone()
        .unwrap_or_else(|| generate_unit_name(&command, cli.scope));
    // Ensure the unit name has a type suffix.
    if !unit_name.contains('.') {
        unit_name.push_str(if cli.scope { ".scope" } else { ".service" });
    }

    // Print unit info to stderr (matching systemd-run behavior)
    print_unit_info(&cli, &unit_name);

    // Try to create a transient unit via the control socket first
    match try_create_transient_unit(&cli, &unit_name) {
        Ok(Some(resp)) => {
            // Successfully created the transient unit.
            if !cli.quiet {
                eprintln!("Running as unit: {unit_name}");
            }

            // When --wait was set, the control socket blocked until the unit
            // finished and the response contains the exit code.
            if cli.wait || cli.pipe {
                // When --pipe, relay captured stdout/stderr to the caller.
                if cli.pipe {
                    if let Some(data) = resp.get("stdout").and_then(|v| v.as_str()) {
                        use std::io::Write;
                        let _ = std::io::stdout().write_all(data.as_bytes());
                    }
                    if let Some(data) = resp.get("stderr").and_then(|v| v.as_str()) {
                        use std::io::Write;
                        let _ = std::io::stderr().write_all(data.as_bytes());
                    }
                }
                let exit_code = resp.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let result = resp
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or("success");
                if result != "success" {
                    // Match systemd-run behavior: non-zero exit propagated
                    process::exit(if exit_code != 0 { exit_code } else { 1 });
                }
                process::exit(exit_code);
            }

            process::exit(0);
        }
        Ok(None) => {
            // Control socket not available — fall back to direct execution
            if command.is_empty() {
                eprintln!("Error: No command to execute and cannot connect to service manager.");
                process::exit(1);
            }

            if !cli.quiet {
                eprintln!(
                    "Note: rust-systemd control socket not available, executing command directly."
                );
            }
        }
        Err(e) => {
            // If the error indicates that the service manager rejected the
            // request (as opposed to a socket-not-found error), exit with
            // failure rather than falling through to direct execution.
            if e.starts_with("Service manager error:") {
                eprintln!("Failed to start transient unit: {e}");
                process::exit(1);
            }
            if !cli.quiet {
                eprintln!("Warning: Failed to create transient unit: {e}");
            }
            if command.is_empty() {
                process::exit(1);
            }
            if !cli.quiet {
                eprintln!("Falling back to direct execution.");
            }
        }
    }

    // === Fallback: Direct execution ===

    // Apply process-level properties
    if let Err(e) = apply_process_properties(&cli) {
        eprintln!("Error: {e}");
        process::exit(1);
    }

    // Build the command
    let cmd = &command[0];
    let args = &command[1..];

    let mut child_cmd = process::Command::new(cmd);
    child_cmd.args(args);

    if cli.wait || cli.pty || cli.pipe {
        // In wait/pty/pipe mode, we spawn and wait
        match child_cmd.spawn() {
            Ok(mut child) => match child.wait() {
                Ok(status) => {
                    process::exit(status.code().unwrap_or(1));
                }
                Err(e) => {
                    eprintln!("Error waiting for {cmd}: {e}");
                    process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("Error executing {cmd}: {e}");
                process::exit(1);
            }
        }
    } else {
        // Default: exec into the command (replaces the current process)
        let err = child_cmd.exec();
        // exec() only returns on error
        eprintln!("Error executing {cmd}: {err}");
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_unit_name_service() {
        let cmd = vec!["echo".to_string(), "hello".to_string()];
        let name = generate_unit_name(&cmd, false);
        assert!(name.starts_with("run-echo"));
        assert!(name.ends_with(".service"));
    }

    #[test]
    fn test_generate_unit_name_scope() {
        let cmd = vec!["/usr/bin/sleep".to_string(), "10".to_string()];
        let name = generate_unit_name(&cmd, true);
        assert!(name.starts_with("run-sleep"));
        assert!(name.ends_with(".scope"));
    }

    #[test]
    fn test_generate_unit_name_empty_command() {
        let cmd: Vec<String> = vec![];
        let name = generate_unit_name(&cmd, false);
        assert!(name.starts_with("run-"));
        assert!(name.ends_with(".service"));
    }

    #[test]
    fn test_generate_unit_name_sanitizes() {
        let cmd = vec!["my program!.sh".to_string()];
        let name = generate_unit_name(&cmd, false);
        // Spaces and exclamation marks should be replaced with underscores
        assert!(name.starts_with("run-my_program__sh"));
        assert!(!name.contains(' '));
        assert!(!name.contains('!'));
    }

    #[test]
    fn test_generate_unit_name_path_command() {
        let cmd = vec!["/usr/local/bin/my-daemon".to_string()];
        let name = generate_unit_name(&cmd, false);
        // Should use only the basename
        assert!(name.starts_with("run-my-daemon"));
        assert!(!name.contains("usr"));
    }

    #[test]
    fn test_default_shell() {
        let shell = default_shell();
        assert!(!shell.is_empty());
        // Should be a path
        assert!(shell.starts_with('/') || shell.contains("sh"));
    }

    #[test]
    fn test_lookup_user_numeric() {
        let result = lookup_user("0");
        assert!(result.is_ok());
        let (uid, _, _, _) = result.unwrap();
        assert_eq!(uid, 0);
    }

    #[test]
    fn test_lookup_user_root() {
        let result = lookup_user("root");
        // root should exist on any Linux system
        assert!(result.is_ok());
        let (uid, _, _, _) = result.unwrap();
        assert_eq!(uid, 0);
    }

    #[test]
    fn test_lookup_user_nonexistent() {
        let result = lookup_user("nonexistent_user_zzz_xyz_12345");
        assert!(result.is_err());
    }

    #[test]
    fn test_lookup_group_numeric() {
        let result = lookup_group("0");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_lookup_group_root() {
        let result = lookup_group("root");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_lookup_group_nonexistent() {
        let result = lookup_group("nonexistent_group_zzz_xyz_12345");
        assert!(result.is_err());
    }
}
