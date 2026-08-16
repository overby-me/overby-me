//! homectl — home directory management CLI
//!
//! This is a Rust implementation of systemd's `homectl` tool. It communicates
//! with the `systemd-homed` daemon via a Unix domain control socket at
//! `/run/systemd/homed-control`. When the daemon is unavailable, it falls
//! back to reading identity files directly from `/var/lib/systemd/home/`.
//!
//! ## Commands
//!
//! - `list`                        — list managed home directories
//! - `inspect USER`                — show detailed user record
//! - `show USER`                   — show user properties (key=value)
//! - `create USER [OPTIONS]`       — create a new managed home
//! - `remove USER`                 — remove a managed home
//! - `activate USER`               — activate (mount) a home directory
//! - `deactivate USER`             — deactivate (unmount) a home directory
//! - `update USER [OPTIONS]`       — update user record
//! - `passwd USER`                 — change user password
//! - `resize USER SIZE`            — resize home area
//! - `lock USER`                   — lock home (for suspend)
//! - `unlock USER`                 — unlock home (after resume)
//! - `lock-all`                    — lock all active homes
//! - `deactivate-all`              — deactivate all active homes
//! - `with USER [-- CMD...]`       — activate, run command, deactivate
//! - `recovery-key USER`           — generate a recovery key
//! - `check-password PASSWORD`     — check password quality
//!
//! ## Create Options
//!
//! - `--real-name=NAME`            — real name for user
//! - `--shell=PATH`                — login shell
//! - `--storage=TYPE`              — storage backend (directory/subvolume/luks/cifs/fscrypt)
//! - `--password=PASS`             — initial password
//! - `--home-dir=PATH`             — home directory path
//! - `--image-path=PATH`           — image/backing store path
//! - `--disk-size=SIZE`            — disk size (K/M/G/T suffixes)
//! - `--cifs-service=//SRV/SHARE`  — CIFS service path (for cifs storage)
//! - `--cifs-user-name=USER`       — CIFS user name
//! - `--cifs-domain=DOMAIN`        — CIFS domain
//! - `--no-password-quality`       — skip password quality checks
//! - `--pkcs11-token-uri=URI`      — PKCS#11 token URI for authentication
//! - `--fido2-device=PATH`         — FIDO2 authenticator device
//!
//! ## Flags
//!
//! - `-H`, `--host`                — (accepted, ignored)
//! - `-M`, `--machine`             — (accepted, ignored)
//! - `--no-pager`                  — (accepted, ignored)
//! - `--no-legend`                 — (accepted, ignored)
//! - `--no-ask-password`           — (accepted, ignored)
//! - `-j`, `--json`                — (accepted, ignored)
//! - `-h`, `--help`                — show usage
//! - `--version`                   — show version

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CONTROL_SOCKET_PATH: &str = "/run/systemd/homed-control";
const IDENTITY_DIR: &str = "/var/lib/systemd/home";
const VERSION: &str = "256";

// ---------------------------------------------------------------------------
// Command parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    List,
    Inspect(String),
    Show(String),
    Create(CreateOpts),
    Remove(String),
    Activate(String),
    Deactivate(String),
    Update(UpdateOpts),
    Passwd(String),
    Resize(String, String),
    Lock(String),
    Unlock(String),
    LockAll,
    DeactivateAll,
    With(String, Vec<String>),
    RecoveryKey(String),
    CheckPassword(String),
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CreateOpts {
    pub user_name: String,
    pub real_name: Option<String>,
    pub shell: Option<String>,
    pub storage: Option<String>,
    pub password: Option<String>,
    pub home_dir: Option<String>,
    pub image_path: Option<String>,
    pub disk_size: Option<String>,
    pub cifs_service: Option<String>,
    pub cifs_user_name: Option<String>,
    pub cifs_domain: Option<String>,
    pub no_password_quality: bool,
    pub pkcs11_token_uri: Option<String>,
    pub fido2_device: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct UpdateOpts {
    pub user_name: String,
    pub real_name: Option<String>,
    pub shell: Option<String>,
    pub password_hint: Option<String>,
    pub auto_login: Option<String>,
}

/// Parse command line arguments into a Command.
pub fn parse_command(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        return Ok(Command::List);
    }

    // Strip known flags and collect positional args
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--version" => return Ok(Command::Version),
            "--no-pager" | "--no-legend" | "--no-ask-password" | "-j" | "--json" => {
                i += 1;
                continue;
            }
            "-H" | "--host" | "-M" | "--machine" => {
                // Skip next arg too (value)
                i += 2;
                continue;
            }
            _ => {
                if arg.starts_with('-') && !arg.starts_with("--") && arg.len() == 2 {
                    // Unknown short flag — skip
                    i += 1;
                    continue;
                }
                positional.push(arg.clone());
            }
        }
        i += 1;
    }

    if positional.is_empty() {
        return Ok(Command::List);
    }

    let verb = positional[0].as_str();
    let rest = &positional[1..];

    match verb {
        "list" => Ok(Command::List),

        "inspect" => {
            if rest.is_empty() {
                return Err("inspect requires a user name".to_string());
            }
            Ok(Command::Inspect(rest[0].clone()))
        }

        "show" => {
            if rest.is_empty() {
                return Err("show requires a user name".to_string());
            }
            Ok(Command::Show(rest[0].clone()))
        }

        "create" => {
            if rest.is_empty() {
                return Err("create requires a user name".to_string());
            }
            let mut opts = CreateOpts {
                user_name: rest[0].clone(),
                ..Default::default()
            };
            parse_create_flags(&rest[1..], &mut opts);
            Ok(Command::Create(opts))
        }

        "remove" => {
            if rest.is_empty() {
                return Err("remove requires a user name".to_string());
            }
            Ok(Command::Remove(rest[0].clone()))
        }

        "activate" => {
            if rest.is_empty() {
                return Err("activate requires a user name".to_string());
            }
            Ok(Command::Activate(rest[0].clone()))
        }

        "deactivate" => {
            if rest.is_empty() {
                return Err("deactivate requires a user name".to_string());
            }
            Ok(Command::Deactivate(rest[0].clone()))
        }

        "update" => {
            if rest.is_empty() {
                return Err("update requires a user name".to_string());
            }
            let mut opts = UpdateOpts {
                user_name: rest[0].clone(),
                ..Default::default()
            };
            parse_update_flags(&rest[1..], &mut opts);
            Ok(Command::Update(opts))
        }

        "passwd" => {
            if rest.is_empty() {
                return Err("passwd requires a user name".to_string());
            }
            Ok(Command::Passwd(rest[0].clone()))
        }

        "resize" => {
            if rest.len() < 2 {
                return Err("resize requires a user name and size".to_string());
            }
            Ok(Command::Resize(rest[0].clone(), rest[1].clone()))
        }

        "lock" => {
            if rest.is_empty() {
                return Err("lock requires a user name".to_string());
            }
            Ok(Command::Lock(rest[0].clone()))
        }

        "unlock" => {
            if rest.is_empty() {
                return Err("unlock requires a user name".to_string());
            }
            Ok(Command::Unlock(rest[0].clone()))
        }

        "lock-all" => Ok(Command::LockAll),

        "deactivate-all" => Ok(Command::DeactivateAll),

        "recovery-key" => {
            if rest.is_empty() {
                return Err("recovery-key requires a user name".to_string());
            }
            Ok(Command::RecoveryKey(rest[0].clone()))
        }

        "check-password" => {
            if rest.is_empty() {
                return Err("check-password requires a password".to_string());
            }
            Ok(Command::CheckPassword(rest[0].clone()))
        }

        "with" => {
            if rest.is_empty() {
                return Err("with requires a user name".to_string());
            }
            let user = rest[0].clone();
            // Everything after "--" is the command
            let mut cmd_args = Vec::new();
            let mut found_separator = false;
            for a in &rest[1..] {
                if a == "--" {
                    found_separator = true;
                    continue;
                }
                if found_separator {
                    cmd_args.push(a.clone());
                }
            }
            Ok(Command::With(user, cmd_args))
        }

        _ => Err(format!("unknown command: {}", verb)),
    }
}

fn parse_create_flags(args: &[String], opts: &mut CreateOpts) {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(val) = a.strip_prefix("--real-name=") {
            opts.real_name = Some(val.to_string());
        } else if a == "--real-name" {
            i += 1;
            if i < args.len() {
                opts.real_name = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--shell=") {
            opts.shell = Some(val.to_string());
        } else if a == "--shell" {
            i += 1;
            if i < args.len() {
                opts.shell = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--storage=") {
            opts.storage = Some(val.to_string());
        } else if a == "--storage" {
            i += 1;
            if i < args.len() {
                opts.storage = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--password=") {
            opts.password = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--home-dir=") {
            opts.home_dir = Some(val.to_string());
        } else if a == "--home-dir" {
            i += 1;
            if i < args.len() {
                opts.home_dir = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--image-path=") {
            opts.image_path = Some(val.to_string());
        } else if a == "--image-path" {
            i += 1;
            if i < args.len() {
                opts.image_path = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--disk-size=") {
            opts.disk_size = Some(val.to_string());
        } else if a == "--disk-size" {
            i += 1;
            if i < args.len() {
                opts.disk_size = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--cifs-service=") {
            opts.cifs_service = Some(val.to_string());
        } else if a == "--cifs-service" {
            i += 1;
            if i < args.len() {
                opts.cifs_service = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--cifs-user-name=") {
            opts.cifs_user_name = Some(val.to_string());
        } else if a == "--cifs-user-name" {
            i += 1;
            if i < args.len() {
                opts.cifs_user_name = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--cifs-domain=") {
            opts.cifs_domain = Some(val.to_string());
        } else if a == "--cifs-domain" {
            i += 1;
            if i < args.len() {
                opts.cifs_domain = Some(args[i].clone());
            }
        } else if a == "--no-password-quality" {
            opts.no_password_quality = true;
        } else if let Some(val) = a.strip_prefix("--pkcs11-token-uri=") {
            opts.pkcs11_token_uri = Some(val.to_string());
        } else if a == "--pkcs11-token-uri" {
            i += 1;
            if i < args.len() {
                opts.pkcs11_token_uri = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--fido2-device=") {
            opts.fido2_device = Some(val.to_string());
        } else if a == "--fido2-device" {
            i += 1;
            if i < args.len() {
                opts.fido2_device = Some(args[i].clone());
            }
        }
        // Silently ignore unknown flags
        i += 1;
    }
}

fn parse_update_flags(args: &[String], opts: &mut UpdateOpts) {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(val) = a.strip_prefix("--real-name=") {
            opts.real_name = Some(val.to_string());
        } else if a == "--real-name" {
            i += 1;
            if i < args.len() {
                opts.real_name = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--shell=") {
            opts.shell = Some(val.to_string());
        } else if a == "--shell" {
            i += 1;
            if i < args.len() {
                opts.shell = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--password-hint=") {
            opts.password_hint = Some(val.to_string());
        } else if a == "--password-hint" {
            i += 1;
            if i < args.len() {
                opts.password_hint = Some(args[i].clone());
            }
        } else if let Some(val) = a.strip_prefix("--auto-login=") {
            opts.auto_login = Some(val.to_string());
        }
        i += 1;
    }
}

// ---------------------------------------------------------------------------
// Daemon communication
// ---------------------------------------------------------------------------

/// Send a command to the homed control socket and return the response.
fn send_command(cmd: &str) -> Result<String, String> {
    send_command_to(CONTROL_SOCKET_PATH, cmd)
}

fn send_command_to(socket_path: &str, cmd: &str) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket_path)
        .map_err(|e| format!("Failed to connect to homed: {}", e))?;

    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

    stream
        .write_all(format!("{}\n", cmd).as_bytes())
        .map_err(|e| format!("Failed to send command: {}", e))?;

    // Shutdown write side so homed knows we're done
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let reader = BufReader::new(&stream);
    let mut response = String::new();
    for line in reader.lines() {
        match line {
            Ok(l) => {
                if !response.is_empty() {
                    response.push('\n');
                }
                response.push_str(&l);
            }
            Err(_) => break,
        }
    }

    Ok(response)
}

// ---------------------------------------------------------------------------
// Offline fallback: read identity files directly
// ---------------------------------------------------------------------------

/// Minimal JSON field extractor — gets string value for a key.
fn json_get_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("\"{}\"", key);
    let pos = json.find(&pattern)?;
    let after_key = &json[pos + pattern.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if let Some(inner) = after_colon.strip_prefix('"') {
        let end = find_unescaped_quote(inner)?;
        Some(&inner[..end])
    } else {
        // Non-string value (number, bool, null)
        let end = after_colon
            .find([',', '}', '\n'])
            .unwrap_or(after_colon.len());
        Some(after_colon[..end].trim())
    }
}

fn find_unescaped_quote(s: &str) -> Option<usize> {
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Read all identity files and produce a list table.
fn offline_list(identity_dir: &Path) -> String {
    let entries = match fs::read_dir(identity_dir) {
        Ok(e) => e,
        Err(_) => return "No managed home directories.\n".to_string(),
    };

    let mut records: Vec<(String, String, String, String, String)> = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".identity") || name.starts_with('.') {
            continue;
        }
        if let Ok(data) = fs::read_to_string(entry.path()) {
            let user = json_get_str(&data, "userName").unwrap_or("?");
            let uid = json_get_str(&data, "uid").unwrap_or("?");
            let state = json_get_str(&data, "state").unwrap_or("inactive");
            let storage = json_get_str(&data, "storage").unwrap_or("directory");
            let home = json_get_str(&data, "homeDirectory").unwrap_or("?");
            records.push((
                user.to_string(),
                uid.to_string(),
                state.to_string(),
                storage.to_string(),
                home.to_string(),
            ));
        }
    }

    if records.is_empty() {
        return "No managed home directories.\n".to_string();
    }

    records.sort_by(|a, b| a.0.cmp(&b.0));

    let mut s = String::new();
    s.push_str(&format!(
        "{:<16} {:>6} {:<12} {:<10} {}\n",
        "NAME", "UID", "STATE", "STORAGE", "HOME"
    ));
    for (user, uid, state, storage, home) in &records {
        s.push_str(&format!(
            "{:<16} {:>6} {:<12} {:<10} {}\n",
            user, uid, state, storage, home
        ));
    }
    s.push_str(&format!("\n{} home(s) listed.\n", records.len()));
    s
}

/// Read a single identity file and return its raw JSON.
fn offline_inspect(identity_dir: &Path, user_name: &str) -> Result<String, String> {
    let path = identity_dir.join(format!("{}.identity", user_name));
    if !path.exists() {
        return Err(format!("No identity record for user '{}'", user_name));
    }
    let data = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    // Format a human-readable summary
    let user = json_get_str(&data, "userName").unwrap_or("?");
    let real = json_get_str(&data, "realName").unwrap_or("?");
    let uid = json_get_str(&data, "uid").unwrap_or("?");
    let gid = json_get_str(&data, "gid").unwrap_or("?");
    let state = json_get_str(&data, "state").unwrap_or("?");
    let storage = json_get_str(&data, "storage").unwrap_or("?");
    let home = json_get_str(&data, "homeDirectory").unwrap_or("?");
    let image = json_get_str(&data, "imagePath").unwrap_or("?");
    let shell = json_get_str(&data, "shell").unwrap_or("?");
    let service = json_get_str(&data, "service").unwrap_or("?");
    let locked = json_get_str(&data, "locked").unwrap_or("false");

    let mut s = String::new();
    s.push_str(&format!("   User name: {}\n", user));
    s.push_str(&format!("   Real name: {}\n", real));
    s.push_str(" Disposition: regular\n");
    s.push_str(&format!("       State: {}\n", state));
    s.push_str(&format!("     Service: {}\n", service));
    s.push_str(&format!(" Home Dir.:  {}\n", home));
    s.push_str(&format!(" Image Path: {}\n", image));
    s.push_str(&format!("     Storage: {}\n", storage));
    s.push_str(&format!("         UID: {}\n", uid));
    s.push_str(&format!("         GID: {}\n", gid));
    s.push_str(&format!("       Shell: {}\n", shell));
    s.push_str(&format!("      Locked: {}\n", locked));
    Ok(s)
}

// ---------------------------------------------------------------------------
// Command execution
// ---------------------------------------------------------------------------

fn build_create_command(opts: &CreateOpts) -> String {
    let mut cmd = format!("CREATE {}", opts.user_name);
    if let Some(ref rn) = opts.real_name {
        cmd.push_str(&format!(" realname={}", rn));
    }
    if let Some(ref sh) = opts.shell {
        cmd.push_str(&format!(" shell={}", sh));
    }
    if let Some(ref st) = opts.storage {
        cmd.push_str(&format!(" storage={}", st));
    }
    if let Some(ref pw) = opts.password {
        cmd.push_str(&format!(" password={}", pw));
    }
    if let Some(ref hd) = opts.home_dir {
        cmd.push_str(&format!(" home={}", hd));
    }
    if let Some(ref ip) = opts.image_path {
        cmd.push_str(&format!(" image={}", ip));
    }
    if let Some(ref ds) = opts.disk_size {
        cmd.push_str(&format!(" disk-size={}", ds));
    }
    if let Some(ref cs) = opts.cifs_service {
        cmd.push_str(&format!(" cifs-service={}", cs));
    }
    if let Some(ref cu) = opts.cifs_user_name {
        cmd.push_str(&format!(" cifs-user={}", cu));
    }
    if let Some(ref cd) = opts.cifs_domain {
        cmd.push_str(&format!(" cifs-domain={}", cd));
    }
    if opts.no_password_quality {
        cmd.push_str(" no-password-quality");
    }
    cmd
}

fn build_update_command(opts: &UpdateOpts) -> String {
    let mut cmd = format!("UPDATE {}", opts.user_name);
    if let Some(ref rn) = opts.real_name {
        cmd.push_str(&format!(" realname={}", rn));
    }
    if let Some(ref sh) = opts.shell {
        cmd.push_str(&format!(" shell={}", sh));
    }
    if let Some(ref ph) = opts.password_hint {
        cmd.push_str(&format!(" password-hint={}", ph));
    }
    if let Some(ref al) = opts.auto_login {
        cmd.push_str(&format!(" auto-login={}", al));
    }
    cmd
}

fn execute_command(cmd: Command) -> i32 {
    match cmd {
        Command::Help => {
            print_usage();
            0
        }
        Command::Version => {
            println!("homectl {}", VERSION);
            0
        }
        Command::RecoveryKey(user) => match send_command(&format!("RECOVERY-KEY {}", user)) {
            Ok(resp) => {
                if resp.starts_with("ERROR:") {
                    eprintln!("{}", resp);
                    1
                } else {
                    print!("{}", resp);
                    if !resp.ends_with('\n') {
                        println!();
                    }
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::CheckPassword(password) => {
            match send_command(&format!("CHECK-PASSWORD {}", password)) {
                Ok(resp) => {
                    print!("{}", resp);
                    if !resp.ends_with('\n') {
                        println!();
                    }
                    if resp.starts_with("OK:") { 0 } else { 1 }
                }
                Err(e) => {
                    eprintln!("Failed to connect to homed: {}", e);
                    1
                }
            }
        }
        Command::List => {
            match send_command("LIST") {
                Ok(resp) => {
                    print!("{}", resp);
                    if !resp.ends_with('\n') {
                        println!();
                    }
                }
                Err(_) => {
                    // Offline fallback
                    print!("{}", offline_list(Path::new(IDENTITY_DIR)));
                }
            }
            0
        }
        Command::Inspect(user) => match send_command(&format!("INSPECT {}", user)) {
            Ok(resp) => {
                if resp.starts_with("ERROR:") {
                    eprintln!("{}", resp);
                    1
                } else {
                    print!("{}", resp);
                    if !resp.ends_with('\n') {
                        println!();
                    }
                    0
                }
            }
            Err(_) => match offline_inspect(Path::new(IDENTITY_DIR), &user) {
                Ok(s) => {
                    print!("{}", s);
                    0
                }
                Err(e) => {
                    eprintln!("{}", e);
                    1
                }
            },
        },
        Command::Show(user) => match send_command(&format!("SHOW {}", user)) {
            Ok(resp) => {
                if resp.starts_with("ERROR:") {
                    eprintln!("{}", resp);
                    1
                } else {
                    print!("{}", resp);
                    if !resp.ends_with('\n') {
                        println!();
                    }
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::Create(opts) => {
            let cmd = build_create_command(&opts);
            match send_command(&cmd) {
                Ok(resp) => {
                    if resp.starts_with("ERROR:") {
                        eprintln!("{}", resp);
                        1
                    } else {
                        println!("{}", resp);
                        0
                    }
                }
                Err(e) => {
                    eprintln!("Failed to connect to homed: {}", e);
                    1
                }
            }
        }
        Command::Remove(user) => match send_command(&format!("REMOVE {}", user)) {
            Ok(resp) => {
                if resp.starts_with("ERROR:") {
                    eprintln!("{}", resp);
                    1
                } else {
                    println!("{}", resp);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::Activate(user) => match send_command(&format!("ACTIVATE {}", user)) {
            Ok(resp) => {
                if resp.starts_with("ERROR:") {
                    eprintln!("{}", resp);
                    1
                } else {
                    println!("{}", resp);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::Deactivate(user) => match send_command(&format!("DEACTIVATE {}", user)) {
            Ok(resp) => {
                if resp.starts_with("ERROR:") {
                    eprintln!("{}", resp);
                    1
                } else {
                    println!("{}", resp);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::Update(opts) => {
            let cmd = build_update_command(&opts);
            match send_command(&cmd) {
                Ok(resp) => {
                    if resp.starts_with("ERROR:") {
                        eprintln!("{}", resp);
                        1
                    } else {
                        println!("{}", resp);
                        0
                    }
                }
                Err(e) => {
                    eprintln!("Failed to connect to homed: {}", e);
                    1
                }
            }
        }
        Command::Passwd(user) => {
            // In a real implementation, we'd prompt for the password interactively.
            // For now, read from HOMECTL_PASSWORD env var or fail.
            let pw = match env::var("HOMECTL_PASSWORD") {
                Ok(p) => p,
                Err(_) => {
                    eprintln!(
                        "Password must be provided via HOMECTL_PASSWORD environment variable"
                    );
                    eprintln!("(interactive prompting not yet implemented)");
                    return 1;
                }
            };
            match send_command(&format!("PASSWD {} {}", user, pw)) {
                Ok(resp) => {
                    if resp.starts_with("ERROR:") {
                        eprintln!("{}", resp);
                        1
                    } else {
                        println!("{}", resp);
                        0
                    }
                }
                Err(e) => {
                    eprintln!("Failed to connect to homed: {}", e);
                    1
                }
            }
        }
        Command::Resize(user, size) => match send_command(&format!("RESIZE {} {}", user, size)) {
            Ok(resp) => {
                if resp.starts_with("ERROR:") {
                    eprintln!("{}", resp);
                    1
                } else {
                    println!("{}", resp);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::Lock(user) => match send_command(&format!("LOCK {}", user)) {
            Ok(resp) => {
                if resp.starts_with("ERROR:") {
                    eprintln!("{}", resp);
                    1
                } else {
                    println!("{}", resp);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::Unlock(user) => match send_command(&format!("UNLOCK {}", user)) {
            Ok(resp) => {
                if resp.starts_with("ERROR:") {
                    eprintln!("{}", resp);
                    1
                } else {
                    println!("{}", resp);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::LockAll => match send_command("LOCK-ALL") {
            Ok(resp) => {
                println!("{}", resp);
                0
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::DeactivateAll => match send_command("DEACTIVATE-ALL") {
            Ok(resp) => {
                println!("{}", resp);
                0
            }
            Err(e) => {
                eprintln!("Failed to connect to homed: {}", e);
                1
            }
        },
        Command::With(user, cmd_args) => {
            // Activate
            match send_command(&format!("ACTIVATE {}", user)) {
                Ok(resp) if resp.starts_with("ERROR:") => {
                    eprintln!("Failed to activate: {}", resp);
                    return 1;
                }
                Err(e) => {
                    eprintln!("Failed to connect to homed: {}", e);
                    return 1;
                }
                _ => {}
            }

            // Run command (or shell)
            let status = if cmd_args.is_empty() {
                let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
                process::Command::new(&shell).status()
            } else {
                process::Command::new(&cmd_args[0])
                    .args(&cmd_args[1..])
                    .status()
            };

            let exit_code = match status {
                Ok(s) => s.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("Failed to run command: {}", e);
                    1
                }
            };

            // Deactivate
            match send_command(&format!("DEACTIVATE {}", user)) {
                Ok(resp) if resp.starts_with("ERROR:") => {
                    eprintln!("Warning: failed to deactivate: {}", resp);
                }
                Err(e) => {
                    eprintln!("Warning: failed to deactivate: {}", e);
                }
                _ => {}
            }

            exit_code
        }
    }
}

fn print_usage() {
    println!("homectl - Manage home directories");
    println!();
    println!("Usage: homectl [COMMAND] [OPTIONS]");
    println!();
    println!("Commands:");
    println!("  list                      List managed home directories");
    println!("  inspect USER              Show detailed user information");
    println!("  show USER                 Show user properties (key=value)");
    println!("  create USER [OPTS]        Create a new managed home");
    println!("  remove USER               Remove a managed home");
    println!("  activate USER             Activate (mount) a home directory");
    println!("  deactivate USER           Deactivate (unmount) a home directory");
    println!("  update USER [OPTS]        Update user record");
    println!("  passwd USER               Change user password");
    println!("  resize USER SIZE          Resize home area");
    println!("  lock USER                 Lock home (for suspend)");
    println!("  unlock USER               Unlock home (after resume)");
    println!("  lock-all                  Lock all active homes");
    println!("  deactivate-all            Deactivate all active homes");
    println!("  with USER [-- CMD...]     Activate, run command, deactivate");
    println!("  recovery-key USER         Generate a recovery key");
    println!("  check-password PASSWORD   Check password quality");
    println!();
    println!("Create Options:");
    println!("  --real-name=NAME          Set real name");
    println!("  --shell=SHELL             Set login shell");
    println!(
        "  --storage=TYPE            Set storage type (directory, subvolume, luks, cifs, fscrypt)"
    );
    println!("  --password=PASSWORD       Set initial password");
    println!("  --home-dir=PATH           Set home directory path");
    println!("  --image-path=PATH         Set image path");
    println!("  --disk-size=SIZE          Set disk size (K/M/G/T suffixes)");
    println!("  --cifs-service=//S/SHARE  Set CIFS service path (for cifs storage)");
    println!("  --cifs-user-name=USER     Set CIFS user name");
    println!("  --cifs-domain=DOMAIN      Set CIFS domain");
    println!("  --no-password-quality     Skip password quality checks");
    println!("  --pkcs11-token-uri=URI    PKCS#11 token URI for authentication");
    println!("  --fido2-device=PATH       FIDO2 authenticator device");
    println!();
    println!("Update Options:");
    println!("  --real-name=NAME          Update real name");
    println!("  --shell=SHELL             Update login shell");
    println!("  --password-hint=HINT      Set password hint");
    println!("  --auto-login=BOOL         Enable/disable auto-login");
    println!();
    println!("General Options:");
    println!("  -h, --help                Show this help");
    println!("  --version                 Show version");
    println!("  --no-pager                Do not pipe output into a pager");
    println!("  --no-legend               Do not show legend/header");
    println!("  --no-ask-password         Do not ask for passwords");
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    match parse_command(&args) {
        Ok(cmd) => {
            let code = execute_command(cmd);
            process::exit(code);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!("Try 'homectl --help' for usage.");
            process::exit(1);
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    // -----------------------------------------------------------------------
    // parse_command: basic commands
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_empty_is_list() {
        assert_eq!(parse_command(&args(&[])).unwrap(), Command::List);
    }

    #[test]
    fn test_parse_list() {
        assert_eq!(parse_command(&args(&["list"])).unwrap(), Command::List);
    }

    #[test]
    fn test_parse_help() {
        assert_eq!(parse_command(&args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse_command(&args(&["-h"])).unwrap(), Command::Help);
    }

    #[test]
    fn test_parse_version() {
        assert_eq!(
            parse_command(&args(&["--version"])).unwrap(),
            Command::Version
        );
    }

    #[test]
    fn test_parse_inspect() {
        assert_eq!(
            parse_command(&args(&["inspect", "alice"])).unwrap(),
            Command::Inspect("alice".to_string())
        );
    }

    #[test]
    fn test_parse_inspect_missing_user() {
        assert!(parse_command(&args(&["inspect"])).is_err());
    }

    #[test]
    fn test_parse_show() {
        assert_eq!(
            parse_command(&args(&["show", "bob"])).unwrap(),
            Command::Show("bob".to_string())
        );
    }

    #[test]
    fn test_parse_show_missing_user() {
        assert!(parse_command(&args(&["show"])).is_err());
    }

    #[test]
    fn test_parse_create_simple() {
        let cmd = parse_command(&args(&["create", "alice"])).unwrap();
        match cmd {
            Command::Create(opts) => {
                assert_eq!(opts.user_name, "alice");
                assert!(opts.real_name.is_none());
                assert!(opts.shell.is_none());
                assert!(opts.storage.is_none());
                assert!(opts.password.is_none());
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn test_parse_create_with_options() {
        let cmd = parse_command(&args(&[
            "create",
            "alice",
            "--real-name=Alice Smith",
            "--shell=/bin/zsh",
            "--storage=directory",
            "--password=secret",
            "--home-dir=/home/alice",
            "--image-path=/home/alice.homedir",
        ]))
        .unwrap();
        match cmd {
            Command::Create(opts) => {
                assert_eq!(opts.user_name, "alice");
                assert_eq!(opts.real_name, Some("Alice Smith".to_string()));
                assert_eq!(opts.shell, Some("/bin/zsh".to_string()));
                assert_eq!(opts.storage, Some("directory".to_string()));
                assert_eq!(opts.password, Some("secret".to_string()));
                assert_eq!(opts.home_dir, Some("/home/alice".to_string()));
                assert_eq!(opts.image_path, Some("/home/alice.homedir".to_string()));
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn test_parse_create_with_space_separated_options() {
        let cmd = parse_command(&args(&[
            "create",
            "alice",
            "--real-name",
            "Alice",
            "--shell",
            "/bin/zsh",
        ]))
        .unwrap();
        match cmd {
            Command::Create(opts) => {
                assert_eq!(opts.real_name, Some("Alice".to_string()));
                assert_eq!(opts.shell, Some("/bin/zsh".to_string()));
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn test_parse_create_missing_user() {
        assert!(parse_command(&args(&["create"])).is_err());
    }

    #[test]
    fn test_parse_remove() {
        assert_eq!(
            parse_command(&args(&["remove", "alice"])).unwrap(),
            Command::Remove("alice".to_string())
        );
    }

    #[test]
    fn test_parse_remove_missing_user() {
        assert!(parse_command(&args(&["remove"])).is_err());
    }

    #[test]
    fn test_parse_activate() {
        assert_eq!(
            parse_command(&args(&["activate", "alice"])).unwrap(),
            Command::Activate("alice".to_string())
        );
    }

    #[test]
    fn test_parse_activate_missing_user() {
        assert!(parse_command(&args(&["activate"])).is_err());
    }

    #[test]
    fn test_parse_deactivate() {
        assert_eq!(
            parse_command(&args(&["deactivate", "alice"])).unwrap(),
            Command::Deactivate("alice".to_string())
        );
    }

    #[test]
    fn test_parse_deactivate_missing_user() {
        assert!(parse_command(&args(&["deactivate"])).is_err());
    }

    #[test]
    fn test_parse_update_simple() {
        let cmd = parse_command(&args(&["update", "alice"])).unwrap();
        match cmd {
            Command::Update(opts) => {
                assert_eq!(opts.user_name, "alice");
                assert!(opts.real_name.is_none());
                assert!(opts.shell.is_none());
            }
            _ => panic!("expected Update"),
        }
    }

    #[test]
    fn test_parse_update_with_options() {
        let cmd = parse_command(&args(&[
            "update",
            "alice",
            "--real-name=Alice",
            "--shell=/bin/fish",
            "--password-hint=color",
            "--auto-login=true",
        ]))
        .unwrap();
        match cmd {
            Command::Update(opts) => {
                assert_eq!(opts.user_name, "alice");
                assert_eq!(opts.real_name, Some("Alice".to_string()));
                assert_eq!(opts.shell, Some("/bin/fish".to_string()));
                assert_eq!(opts.password_hint, Some("color".to_string()));
                assert_eq!(opts.auto_login, Some("true".to_string()));
            }
            _ => panic!("expected Update"),
        }
    }

    #[test]
    fn test_parse_update_missing_user() {
        assert!(parse_command(&args(&["update"])).is_err());
    }

    #[test]
    fn test_parse_passwd() {
        assert_eq!(
            parse_command(&args(&["passwd", "alice"])).unwrap(),
            Command::Passwd("alice".to_string())
        );
    }

    #[test]
    fn test_parse_passwd_missing_user() {
        assert!(parse_command(&args(&["passwd"])).is_err());
    }

    #[test]
    fn test_parse_resize() {
        assert_eq!(
            parse_command(&args(&["resize", "alice", "10G"])).unwrap(),
            Command::Resize("alice".to_string(), "10G".to_string())
        );
    }

    #[test]
    fn test_parse_resize_missing_args() {
        assert!(parse_command(&args(&["resize"])).is_err());
        assert!(parse_command(&args(&["resize", "alice"])).is_err());
    }

    #[test]
    fn test_parse_lock() {
        assert_eq!(
            parse_command(&args(&["lock", "alice"])).unwrap(),
            Command::Lock("alice".to_string())
        );
    }

    #[test]
    fn test_parse_lock_missing_user() {
        assert!(parse_command(&args(&["lock"])).is_err());
    }

    #[test]
    fn test_parse_unlock() {
        assert_eq!(
            parse_command(&args(&["unlock", "alice"])).unwrap(),
            Command::Unlock("alice".to_string())
        );
    }

    #[test]
    fn test_parse_unlock_missing_user() {
        assert!(parse_command(&args(&["unlock"])).is_err());
    }

    #[test]
    fn test_parse_lock_all() {
        assert_eq!(
            parse_command(&args(&["lock-all"])).unwrap(),
            Command::LockAll
        );
    }

    #[test]
    fn test_parse_deactivate_all() {
        assert_eq!(
            parse_command(&args(&["deactivate-all"])).unwrap(),
            Command::DeactivateAll
        );
    }

    #[test]
    fn test_parse_with_no_command() {
        let cmd = parse_command(&args(&["with", "alice"])).unwrap();
        match cmd {
            Command::With(user, cmd_args) => {
                assert_eq!(user, "alice");
                assert!(cmd_args.is_empty());
            }
            _ => panic!("expected With"),
        }
    }

    #[test]
    fn test_parse_with_command() {
        let cmd = parse_command(&args(&["with", "alice", "--", "ls", "-la"])).unwrap();
        match cmd {
            Command::With(user, cmd_args) => {
                assert_eq!(user, "alice");
                assert_eq!(cmd_args, vec!["ls".to_string(), "-la".to_string()]);
            }
            _ => panic!("expected With"),
        }
    }

    #[test]
    fn test_parse_with_missing_user() {
        assert!(parse_command(&args(&["with"])).is_err());
    }

    #[test]
    fn test_parse_unknown_command() {
        assert!(parse_command(&args(&["foobar"])).is_err());
    }

    // -----------------------------------------------------------------------
    // parse_command: flag stripping
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_strip_no_pager() {
        assert_eq!(
            parse_command(&args(&["--no-pager", "list"])).unwrap(),
            Command::List
        );
    }

    #[test]
    fn test_parse_strip_no_legend() {
        assert_eq!(
            parse_command(&args(&["--no-legend", "list"])).unwrap(),
            Command::List
        );
    }

    #[test]
    fn test_parse_strip_no_ask_password() {
        assert_eq!(
            parse_command(&args(&["--no-ask-password", "list"])).unwrap(),
            Command::List
        );
    }

    #[test]
    fn test_parse_strip_json_flag() {
        assert_eq!(
            parse_command(&args(&["-j", "list"])).unwrap(),
            Command::List
        );
        assert_eq!(
            parse_command(&args(&["--json", "list"])).unwrap(),
            Command::List
        );
    }

    #[test]
    fn test_parse_strip_host_flag() {
        assert_eq!(
            parse_command(&args(&["-H", "myhost", "list"])).unwrap(),
            Command::List
        );
        assert_eq!(
            parse_command(&args(&["--host", "myhost", "list"])).unwrap(),
            Command::List
        );
    }

    #[test]
    fn test_parse_strip_machine_flag() {
        assert_eq!(
            parse_command(&args(&["-M", "myvm", "list"])).unwrap(),
            Command::List
        );
        assert_eq!(
            parse_command(&args(&["--machine", "myvm", "list"])).unwrap(),
            Command::List
        );
    }

    #[test]
    fn test_parse_help_takes_priority() {
        assert_eq!(
            parse_command(&args(&["list", "--help"])).unwrap(),
            Command::Help
        );
        assert_eq!(
            parse_command(&args(&["-h", "create", "alice"])).unwrap(),
            Command::Help
        );
    }

    #[test]
    fn test_parse_version_takes_priority() {
        assert_eq!(
            parse_command(&args(&["list", "--version"])).unwrap(),
            Command::Version
        );
    }

    #[test]
    fn test_parse_only_flags_is_list() {
        assert_eq!(
            parse_command(&args(&["--no-pager", "--no-legend"])).unwrap(),
            Command::List
        );
    }

    // -----------------------------------------------------------------------
    // build_create_command
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_create_command_simple() {
        let opts = CreateOpts {
            user_name: "alice".to_string(),
            ..Default::default()
        };
        let cmd = build_create_command(&opts);
        assert_eq!(cmd, "CREATE alice");
    }

    #[test]
    fn test_build_create_command_full() {
        let opts = CreateOpts {
            user_name: "alice".to_string(),
            real_name: Some("Alice".to_string()),
            shell: Some("/bin/zsh".to_string()),
            storage: Some("luks".to_string()),
            password: Some("secret".to_string()),
            home_dir: Some("/home/alice".to_string()),
            image_path: Some("/home/alice.home".to_string()),
            disk_size: Some("10G".to_string()),
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            no_password_quality: false,
            pkcs11_token_uri: None,
            fido2_device: None,
        };
        let cmd = build_create_command(&opts);
        assert!(cmd.starts_with("CREATE alice"));
        assert!(cmd.contains("realname=Alice"));
        assert!(cmd.contains("shell=/bin/zsh"));
        assert!(cmd.contains("storage=luks"));
        assert!(cmd.contains("password=secret"));
        assert!(cmd.contains("home=/home/alice"));
        assert!(cmd.contains("image=/home/alice.home"));
        assert!(cmd.contains("disk-size=10G"));
    }

    #[test]
    fn test_build_create_command_cifs() {
        let opts = CreateOpts {
            user_name: "bob".to_string(),
            storage: Some("cifs".to_string()),
            cifs_service: Some("//server/share".to_string()),
            cifs_user_name: Some("netbob".to_string()),
            cifs_domain: Some("CORP".to_string()),
            no_password_quality: true,
            ..Default::default()
        };
        let cmd = build_create_command(&opts);
        assert!(cmd.starts_with("CREATE bob"));
        assert!(cmd.contains("storage=cifs"));
        assert!(cmd.contains("cifs-service=//server/share"));
        assert!(cmd.contains("cifs-user=netbob"));
        assert!(cmd.contains("cifs-domain=CORP"));
        assert!(cmd.contains("no-password-quality"));
    }

    // -----------------------------------------------------------------------
    // build_update_command
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_update_command_simple() {
        let opts = UpdateOpts {
            user_name: "alice".to_string(),
            ..Default::default()
        };
        assert_eq!(build_update_command(&opts), "UPDATE alice");
    }

    #[test]
    fn test_build_update_command_full() {
        let opts = UpdateOpts {
            user_name: "alice".to_string(),
            real_name: Some("Alice".to_string()),
            shell: Some("/bin/fish".to_string()),
            password_hint: Some("color".to_string()),
            auto_login: Some("true".to_string()),
        };
        let cmd = build_update_command(&opts);
        assert!(cmd.starts_with("UPDATE alice"));
        assert!(cmd.contains("realname=Alice"));
        assert!(cmd.contains("shell=/bin/fish"));
        assert!(cmd.contains("password-hint=color"));
        assert!(cmd.contains("auto-login=true"));
    }

    // -----------------------------------------------------------------------
    // json_get_str
    // -----------------------------------------------------------------------

    #[test]
    fn test_json_get_str_basic() {
        let json = r#"{"userName": "alice", "uid": 60001}"#;
        assert_eq!(json_get_str(json, "userName"), Some("alice"));
        assert_eq!(json_get_str(json, "uid"), Some("60001"));
    }

    #[test]
    fn test_json_get_str_missing() {
        let json = r#"{"userName": "alice"}"#;
        assert_eq!(json_get_str(json, "nonexistent"), None);
    }

    #[test]
    fn test_json_get_str_null() {
        let json = r#"{"diskSize": null}"#;
        assert_eq!(json_get_str(json, "diskSize"), Some("null"));
    }

    #[test]
    fn test_json_get_str_boolean() {
        let json = r#"{"locked": false, "autoLogin": true}"#;
        assert_eq!(json_get_str(json, "locked"), Some("false"));
        assert_eq!(json_get_str(json, "autoLogin"), Some("true"));
    }

    #[test]
    fn test_json_get_str_with_whitespace() {
        let json = "{\n  \"userName\": \"alice\"\n}";
        assert_eq!(json_get_str(json, "userName"), Some("alice"));
    }

    // -----------------------------------------------------------------------
    // find_unescaped_quote
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_unescaped_quote_simple() {
        assert_eq!(find_unescaped_quote("hello\""), Some(5));
    }

    #[test]
    fn test_find_unescaped_quote_escaped() {
        assert_eq!(find_unescaped_quote("hello\\\"world\""), Some(12));
    }

    #[test]
    fn test_find_unescaped_quote_none() {
        assert_eq!(find_unescaped_quote("no quote here"), None);
    }

    #[test]
    fn test_find_unescaped_quote_at_start() {
        assert_eq!(find_unescaped_quote("\""), Some(0));
    }

    // -----------------------------------------------------------------------
    // Offline fallback: list
    // -----------------------------------------------------------------------

    #[test]
    fn test_offline_list_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let s = offline_list(tmp.path());
        assert!(s.contains("No managed home directories"));
    }

    #[test]
    fn test_offline_list_nonexistent_dir() {
        let s = offline_list(Path::new("/tmp/nonexistent_homectl_test_dir"));
        assert!(s.contains("No managed home directories"));
    }

    #[test]
    fn test_offline_list_with_records() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("alice.identity"),
            r#"{"userName": "alice", "uid": 60001, "state": "active", "storage": "directory", "homeDirectory": "/home/alice"}"#,
        ).unwrap();
        fs::write(
            tmp.path().join("bob.identity"),
            r#"{"userName": "bob", "uid": 60002, "state": "inactive", "storage": "luks", "homeDirectory": "/home/bob"}"#,
        ).unwrap();

        let s = offline_list(tmp.path());
        assert!(s.contains("alice"));
        assert!(s.contains("bob"));
        assert!(s.contains("2 home(s) listed"));
        assert!(s.contains("60001"));
        assert!(s.contains("active"));
        assert!(s.contains("luks"));
    }

    #[test]
    fn test_offline_list_skips_dotfiles() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("alice.identity"),
            r#"{"userName": "alice", "uid": 60001}"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join(".hidden.identity"),
            r#"{"userName": "hidden", "uid": 60099}"#,
        )
        .unwrap();
        fs::write(tmp.path().join("readme.txt"), "not an identity file").unwrap();

        let s = offline_list(tmp.path());
        assert!(s.contains("alice"));
        assert!(!s.contains("hidden"));
        assert!(s.contains("1 home(s) listed"));
    }

    #[test]
    fn test_offline_list_sorted() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("charlie.identity"),
            r#"{"userName": "charlie", "uid": 60003}"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("alice.identity"),
            r#"{"userName": "alice", "uid": 60001}"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("bob.identity"),
            r#"{"userName": "bob", "uid": 60002}"#,
        )
        .unwrap();

        let s = offline_list(tmp.path());
        let alice_pos = s.find("alice").unwrap();
        let bob_pos = s.find("bob").unwrap();
        let charlie_pos = s.find("charlie").unwrap();
        assert!(alice_pos < bob_pos);
        assert!(bob_pos < charlie_pos);
    }

    // -----------------------------------------------------------------------
    // Offline fallback: inspect
    // -----------------------------------------------------------------------

    #[test]
    fn test_offline_inspect_found() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("alice.identity"),
            r#"{
  "userName": "alice",
  "realName": "Alice Smith",
  "uid": 60001,
  "gid": 60001,
  "state": "active",
  "storage": "directory",
  "homeDirectory": "/home/alice",
  "imagePath": "/home/alice.homedir",
  "shell": "/bin/bash",
  "service": "io.systemd.Home",
  "locked": false
}"#,
        )
        .unwrap();

        let s = offline_inspect(tmp.path(), "alice").unwrap();
        assert!(s.contains("alice"));
        assert!(s.contains("Alice Smith"));
        assert!(s.contains("60001"));
        assert!(s.contains("active"));
        assert!(s.contains("directory"));
        assert!(s.contains("/home/alice"));
        assert!(s.contains("io.systemd.Home"));
    }

    #[test]
    fn test_offline_inspect_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = offline_inspect(tmp.path(), "ghost");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No identity record"));
    }

    // -----------------------------------------------------------------------
    // Integration: parse → build command string
    // -----------------------------------------------------------------------

    #[test]
    fn test_roundtrip_create_command() {
        let cmd = parse_command(&args(&[
            "create",
            "alice",
            "--real-name=Alice",
            "--shell=/bin/zsh",
            "--storage=directory",
        ]))
        .unwrap();
        match cmd {
            Command::Create(opts) => {
                let s = build_create_command(&opts);
                assert!(s.contains("CREATE alice"));
                assert!(s.contains("realname=Alice"));
                assert!(s.contains("shell=/bin/zsh"));
                assert!(s.contains("storage=directory"));
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn test_roundtrip_update_command() {
        let cmd = parse_command(&args(&[
            "update",
            "bob",
            "--real-name=Robert",
            "--auto-login=false",
        ]))
        .unwrap();
        match cmd {
            Command::Update(opts) => {
                let s = build_update_command(&opts);
                assert!(s.contains("UPDATE bob"));
                assert!(s.contains("realname=Robert"));
                assert!(s.contains("auto-login=false"));
            }
            _ => panic!("expected Update"),
        }
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_multiple_flags_stripped() {
        assert_eq!(
            parse_command(&args(&[
                "--no-pager",
                "--no-legend",
                "--no-ask-password",
                "-j",
                "list"
            ]))
            .unwrap(),
            Command::List
        );
    }

    #[test]
    fn test_parse_with_empty_separator() {
        // "with alice --" with no command after separator
        let cmd = parse_command(&args(&["with", "alice", "--"])).unwrap();
        match cmd {
            Command::With(user, cmd_args) => {
                assert_eq!(user, "alice");
                assert!(cmd_args.is_empty());
            }
            _ => panic!("expected With"),
        }
    }

    #[test]
    fn test_parse_resize_large_size() {
        assert_eq!(
            parse_command(&args(&["resize", "alice", "100T"])).unwrap(),
            Command::Resize("alice".to_string(), "100T".to_string())
        );
    }

    #[test]
    fn test_parse_create_only_image_path() {
        let cmd = parse_command(&args(&[
            "create",
            "alice",
            "--image-path=/tmp/alice.homedir",
        ]))
        .unwrap();
        match cmd {
            Command::Create(opts) => {
                assert_eq!(opts.image_path, Some("/tmp/alice.homedir".to_string()));
                assert!(opts.real_name.is_none());
            }
            _ => panic!("expected Create"),
        }
    }
}
