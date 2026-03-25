//! systemctl — CLI control tool for the rust-systemd service manager.
//!
//! This is the successor to `rsdctl`. It packs CLI arguments into the
//! JSON-RPC 2.0 format and sends them to the rust-systemd control socket.
//! It reads the response and pretty-prints it.
//!
//! Handles common systemctl flags (stripping them before sending to PID 1):
//!   --no-block, --quiet, --no-wall, --force, --system, --user,
//!   --no-pager, --no-legend, --no-ask-password, --plain, --full,
//!   --show-types, --failed, --all, -a, -f, -q, -l, -t, -p, -n
//!
//! Also handles special commands:
//!   poweroff, reboot, halt, kexec  → mapped to "shutdown" with verb as param
//!   daemon-reload           → mapped to "reload"
//!   try-restart             → forwarded as "try-restart"
//!   reload-or-restart       → forwarded as "reload-or-restart"
//!   condrestart             → alias for "try-restart"
//!   is-active               → checks unit state, exits 0 if active, 3 if not
//!   is-enabled              → checks unit enablement
//!   is-failed               → checks if unit is in failed state
//!   list-dependencies       → show dependency tree for a unit
//!   list-unit-files         → list all unit files on disk with their state
//!   mask                    → symlink unit files to /dev/null
//!   unmask                  → remove /dev/null symlinks for units
//!   disable                 → disable unit(s) (no-op for now)
//!   reset-failed            → clear failed state of unit(s)
//!   kill                    → send signal to a unit's processes
//!   suspend                 → suspend the system (sleep to RAM)
//!   hibernate               → hibernate the system (sleep to disk)
//!   hybrid-sleep            → hybrid sleep (RAM + disk)
//!   suspend-then-hibernate  → suspend first, then hibernate after delay

use serde_json::Value;
use std::io::Write;

use libsystemd::control::jsonrpc2::Call;

/// Flags we recognize and strip from the argument list before sending
/// the command to PID 1. These are common systemctl flags that don't
/// affect the wire protocol.
const KNOWN_FLAGS: &[&str] = &[
    "--no-block",
    "--quiet",
    "--no-wall",
    "--force",
    "--system",
    "--user",
    "--no-pager",
    "--no-legend",
    "--no-ask-password",
    "--plain",
    "--full",
    "--show-types",
    "--failed",
    "--all",
    "--wait",
    "--now",
    "--runtime",
    "--global",
    "--no-reload",
    "--no-warn",
    "--check-inhibitors=auto",
    "--check-inhibitors=yes",
    "--check-inhibitors=no",
    "--show-transaction",
    "--recursive",
    "--with-dependencies",
    "--dry-run",
    "-T",
];

/// Short flags we recognize and strip.
const KNOWN_SHORT_FLAGS: &[&str] = &["-a", "-f", "-q", "-l"];

/// Short flags that consume the next argument (e.g. `-t service`, `-p MainPID`).
const SHORT_FLAGS_WITH_VALUE: &[&str] = &["-t", "-p", "-n", "-o", "-H", "-M", "-s"];

/// Long flags that consume `=value` or the next argument.
const LONG_FLAGS_WITH_VALUE: &[&str] = &[
    "--type",
    "--property",
    "--lines",
    "--output",
    "--host",
    "--machine",
    "--signal",
    "--kill-mode",
    "--kill-who",
    "--kill-whom",
    "--kill-value",
    "--job-mode",
    "--root",
    "--preset-mode",
    "--what",
    "--legend",
    "--timestamp",
    "--message",
];

fn main() {
    let mut args: Vec<_> = std::env::args().collect();
    let exec_name = args.remove(0);

    // When invoked as "poweroff", "reboot", "halt", or "kexec" (e.g. via
    // symlink), treat it as if "systemctl <verb>" was called.
    let implicit_command: &[&str] = &["poweroff", "reboot", "halt", "kexec"];
    for cmd in implicit_command {
        if exec_name.ends_with(cmd) {
            args.insert(0, cmd.to_string());
            break;
        }
    }

    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_help();
        return;
    }

    if args[0] == "--version" {
        println!("systemctl (rust-systemd) 258");
        return;
    }

    // Determine the control socket address.
    let addr = if let Ok(env_addr) = std::env::var("SYSTEMCTL_ADDR") {
        env_addr
    } else if args.len() >= 2 && (args[0].contains(':') || args[0].starts_with('/')) {
        // First arg looks like an address (host:port or /path/to/socket)
        args.remove(0)
    } else {
        // Default to the rust-systemd control socket
        "/run/systemd/rust-systemd-notify/control.socket".to_owned()
    };

    // Extract known flags and separate them from positional arguments.
    let mut quiet = false;
    let mut no_block = false;
    let mut positional: Vec<String> = Vec::new();
    let mut property_filter: Vec<String> = Vec::new();
    let mut value_only = false;
    let mut state_filter: Option<String> = None;

    let mut force = false;
    let mut wait = false;
    let mut root_path: Option<String> = None;
    let mut runtime = false;
    let mut dry_run = false;
    let mut marked = false;
    let mut output_format: Option<String> = None;
    let mut what_filter: Option<String> = None;
    let mut now = false;
    let mut preset_mode: Option<String> = None;
    let mut kill_whom: Option<String> = None;
    let mut kill_value: Option<i32> = None;

    let mut i = 0;
    let mut end_of_options = false;
    while i < args.len() {
        let arg = &args[i];

        // `--` signals end-of-options: all remaining args are positional.
        if arg == "--" && !end_of_options {
            end_of_options = true;
            i += 1;
            continue;
        }
        if end_of_options {
            positional.push(arg.clone());
            i += 1;
            continue;
        }

        // --what flag (for `clean --what=configuration`)
        if arg == "--what" {
            if i + 1 < args.len() {
                what_filter = Some(args[i + 1].clone());
            }
            i += 2;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--what=") {
            what_filter = Some(rest.to_string());
            i += 1;
            continue;
        }

        // --kill-whom flag (for `kill --kill-whom=main`)
        if arg == "--kill-whom" || arg == "--kill-who" {
            if i + 1 < args.len() {
                kill_whom = Some(args[i + 1].clone());
            }
            i += 2;
            continue;
        }
        if let Some(rest) = arg
            .strip_prefix("--kill-whom=")
            .or_else(|| arg.strip_prefix("--kill-who="))
        {
            kill_whom = Some(rest.to_string());
            i += 1;
            continue;
        }

        // --kill-value flag (for `kill --kill-value=4`)
        if arg == "--kill-value" {
            if i + 1 < args.len() {
                kill_value = args[i + 1].parse::<i32>().ok();
            }
            i += 2;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--kill-value=") {
            kill_value = rest.parse::<i32>().ok();
            i += 1;
            continue;
        }

        // --output flag (for show-environment --output=json)
        if arg == "--output" || arg == "-o" {
            if i + 1 < args.len() {
                output_format = Some(args[i + 1].clone());
            }
            i += 2;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--output=") {
            output_format = Some(rest.to_string());
            i += 1;
            continue;
        }

        // --root flag (for filesystem operations like get-default, set-default)
        if arg == "--root" {
            if i + 1 < args.len() {
                root_path = Some(args[i + 1].clone());
            }
            i += 2;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--root=") {
            root_path = Some(rest.to_string());
            i += 1;
            continue;
        }

        // --preset-mode flag (for preset command)
        if arg == "--preset-mode" {
            if i + 1 < args.len() {
                preset_mode = Some(args[i + 1].clone());
            }
            i += 2;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--preset-mode=") {
            preset_mode = Some(rest.to_string());
            i += 1;
            continue;
        }

        // --value flag (for `show --value -p Prop`)
        if arg == "--value" {
            value_only = true;
            i += 1;
            continue;
        }

        // Check known long flags (exact match).
        if KNOWN_FLAGS.contains(&arg.as_str()) {
            if arg == "--quiet" || arg == "-q" {
                quiet = true;
            }
            if arg == "--no-block" {
                no_block = true;
            }
            if arg == "--force" {
                force = true;
            }
            if arg == "--wait" {
                wait = true;
            }
            if arg == "--runtime" {
                runtime = true;
            }
            if arg == "--dry-run" {
                dry_run = true;
            }
            if arg == "--now" {
                now = true;
            }
            if arg == "--failed" {
                state_filter = Some("failed".to_string());
            }
            i += 1;
            continue;
        }

        // Check known short flags.
        if KNOWN_SHORT_FLAGS.contains(&arg.as_str()) {
            if arg == "-q" {
                quiet = true;
            }
            if arg == "-f" {
                force = true;
            }
            i += 1;
            continue;
        }

        // -P PROP is shorthand for --value --property=PROP
        if arg == "-P" {
            value_only = true;
            if i + 1 < args.len() {
                for part in args[i + 1].split(',') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        property_filter.push(trimmed.to_owned());
                    }
                }
            }
            i += 2;
            continue;
        }

        // Capture -p / --property values for `show` filtering.
        if arg == "-p" || arg == "--property" {
            if i + 1 < args.len() {
                // Value may be comma-separated: -p MainPID,ActiveState
                for part in args[i + 1].split(',') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        property_filter.push(trimmed.to_owned());
                    }
                }
            }
            i += 2;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--property=") {
            for part in rest.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    property_filter.push(trimmed.to_owned());
                }
            }
            i += 1;
            continue;
        }

        // Capture --state value for list-units filtering.
        if arg == "--state" {
            if i + 1 < args.len() {
                state_filter = Some(args[i + 1].clone());
            }
            i += 2;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--state=") {
            state_filter = Some(rest.to_string());
            i += 1;
            continue;
        }

        // Check long flags with value (--type=foo or --type foo).
        let mut matched_long = false;
        for flag in LONG_FLAGS_WITH_VALUE {
            if arg == *flag {
                // Consumes next argument.
                i += 2;
                matched_long = true;
                break;
            }
            if arg.starts_with(&format!("{}=", flag)) {
                i += 1;
                matched_long = true;
                break;
            }
        }
        if matched_long {
            continue;
        }

        // Check short flags with value (-t service).
        if SHORT_FLAGS_WITH_VALUE.contains(&arg.as_str()) {
            // Skip this flag and its value.
            i += 2;
            continue;
        }

        // Not a flag — it's a positional argument.
        positional.push(arg.clone());
        i += 1;
    }

    // When no command is given, real systemd defaults to `list-units`.
    if positional.is_empty() {
        positional.push("list-units".to_string());
    }

    // Extract --signal flag for kill command
    let mut kill_signal: Option<i32> = None;
    {
        let parse_signal_name = |name: &str| -> Option<i32> {
            if let Ok(sig) = name.parse::<i32>() {
                return Some(sig);
            }
            // Handle SIGRTMIN+N / SIGRTMAX-N
            let name_upper = name.to_uppercase();
            if let Some(offset) = name_upper
                .strip_prefix("SIGRTMIN+")
                .or_else(|| name_upper.strip_prefix("RTMIN+"))
                && let Ok(n) = offset.parse::<i32>()
            {
                return Some(34 + n); // SIGRTMIN = 34 on Linux
            }
            if let Some(offset) = name_upper
                .strip_prefix("SIGRTMAX-")
                .or_else(|| name_upper.strip_prefix("RTMAX-"))
                && let Ok(n) = offset.parse::<i32>()
            {
                return Some(64 - n); // SIGRTMAX = 64 on Linux
            }
            match name_upper.as_str() {
                "SIGTERM" | "TERM" => Some(15),
                "SIGKILL" | "KILL" => Some(9),
                "SIGHUP" | "HUP" => Some(1),
                "SIGINT" | "INT" => Some(2),
                "SIGUSR1" | "USR1" => Some(10),
                "SIGUSR2" | "USR2" => Some(12),
                "SIGCONT" | "CONT" => Some(18),
                "SIGSTOP" | "STOP" => Some(19),
                "SIGTSTP" | "TSTP" => Some(20),
                "SIGQUIT" | "QUIT" => Some(3),
                "SIGABRT" | "ABRT" => Some(6),
                "SIGPIPE" | "PIPE" => Some(13),
                "SIGALRM" | "ALRM" => Some(14),
                "SIGCHLD" | "CHLD" => Some(17),
                "SIGWINCH" | "WINCH" => Some(28),
                _ => None,
            }
        };
        let mut i = 0;
        while i < positional.len() {
            if positional[i] == "--signal" || positional[i] == "-s" {
                positional.remove(i);
                if i < positional.len() {
                    kill_signal = parse_signal_name(&positional[i]).or(Some(15));
                    positional.remove(i);
                }
            } else if let Some(rest) = positional[i].strip_prefix("--signal=") {
                kill_signal = parse_signal_name(rest).or(Some(15));
                positional.remove(i);
            } else {
                i += 1;
            }
        }
    }

    // Extract --reverse, --after, --before, --plain flags for list-dependencies
    let mut reverse = false;
    let mut after = false;
    let mut before = false;
    let mut plain = false;
    positional.retain(|arg| match arg.as_str() {
        "--reverse" => {
            reverse = true;
            false
        }
        "--after" => {
            after = true;
            false
        }
        "--before" => {
            before = true;
            false
        }
        "--plain" => {
            plain = true;
            false
        }
        _ => true,
    });

    // Extract --marked flag
    positional.retain(|arg| {
        if arg == "--marked" {
            marked = true;
            false
        } else {
            true
        }
    });

    // Extract --full flag for edit
    let mut full = false;
    positional.retain(|arg| {
        if arg == "--full" {
            full = true;
            false
        } else {
            true
        }
    });

    // Map command aliases.
    let command = match positional[0].as_str() {
        "poweroff" | "reboot" | "halt" | "kexec" => {
            let verb = positional[0].clone();
            positional[0] = "shutdown".to_string();
            // Keep the original verb as a parameter so the server knows
            // which action to perform (poweroff vs reboot vs halt vs kexec).
            if positional.len() < 2 {
                positional.push(verb);
            } else {
                positional.insert(1, verb);
            }
            &positional[0]
        }
        "daemon-reload" => {
            positional[0] = "reload".to_string();
            &positional[0]
        }
        "daemon-reexec" => {
            // Pass through as-is — the control protocol handles it separately
            // from daemon-reload, triggering a full re-execution of the binary.
            &positional[0]
        }
        "condrestart" => {
            positional[0] = "try-restart".to_string();
            &positional[0]
        }
        "force-reload" | "try-reload-or-restart" => {
            positional[0] = "try-restart".to_string();
            &positional[0]
        }
        // Sleep commands — pass through as-is to PID 1
        "suspend" | "hibernate" | "hybrid-sleep" | "suspend-then-hibernate" => &positional[0],
        // Timer, property, edit, revert, clean commands — pass through
        "list-timers" | "list-sockets" | "list-paths" | "list-jobs" | "set-property" | "edit"
        | "revert" | "clean" => &positional[0],
        // log-level, log-target, service-watchdogs — get or set manager properties
        "log-level" | "log-target" | "service-watchdogs" => &positional[0],
        // is-failed with no unit = is-system-running (system state check)
        "is-failed" if positional.len() < 2 => {
            positional[0] = "is-system-running".to_string();
            &positional[0]
        }
        _ => &positional[0],
    };

    // Handle `help` client-side: try to open the man page for the unit type.
    if positional[0] == "help" {
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: help requires a unit name.");
            }
            std::process::exit(1);
        }
        let unit_name = &positional[1];
        // Determine man page from unit suffix
        let man_page = if unit_name.ends_with(".service") || !unit_name.contains('.') {
            "systemd.service"
        } else if unit_name.ends_with(".socket") {
            "systemd.socket"
        } else if unit_name.ends_with(".timer") {
            "systemd.timer"
        } else if unit_name.ends_with(".mount") {
            "systemd.mount"
        } else if unit_name.ends_with(".target") {
            "systemd.target"
        } else if unit_name.ends_with(".path") {
            "systemd.path"
        } else if unit_name.ends_with(".slice") {
            "systemd.slice"
        } else if unit_name.ends_with(".scope") {
            "systemd.scope"
        } else if unit_name.ends_with(".swap") {
            "systemd.swap"
        } else if unit_name.ends_with(".automount") {
            "systemd.automount"
        } else if unit_name.ends_with(".device") {
            "systemd.device"
        } else {
            "systemd.service"
        };
        let status = std::process::Command::new("man").arg(man_page).status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                // man not available or failed; not an error for our purposes
            }
        }
        return;
    }

    // Handle `edit` client-side: query PID 1 for unit info, open editor, then daemon-reload.
    if positional[0] == "edit" {
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: edit requires a unit name.");
            }
            std::process::exit(1);
        }
        let unit_name = &positional[1];

        // Query PID 1 for the unit's fragment path and existing override content.
        let mut query_arr = vec![Value::String(unit_name.clone())];
        if full {
            query_arr.push(Value::String("--full".to_owned()));
        }
        let query_call = Call {
            method: "edit".to_string(),
            params: Some(Value::Array(query_arr)),
            id: None,
        };
        let query_str = serde_json::to_string(&query_call.to_json()).unwrap();
        let query_result = if addr.starts_with('/') {
            send_unix(&addr, &query_str)
        } else {
            send_tcp(&addr, &query_str)
        };

        let info = match query_result {
            Ok(resp) => {
                if let Some(error) = resp.get("error") {
                    let message = error
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    if !quiet {
                        eprintln!("{}", message);
                    }
                    std::process::exit(1);
                }
                resp.get("result").cloned().unwrap_or(Value::Null)
            }
            Err(e) => {
                if !quiet {
                    eprintln!("Error communicating with rust-systemd: {e}");
                }
                std::process::exit(1);
            }
        };

        // Determine the editor.
        let editor = std::env::var("SYSTEMD_EDITOR")
            .or_else(|_| std::env::var("EDITOR"))
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| "vi".to_owned());

        let is_full = info.get("full").and_then(|v| v.as_bool()).unwrap_or(false);

        if is_full {
            // --full mode: edit a full copy of the unit file in /etc/systemd/system/.
            let original = info
                .get("original_content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let etc_path = std::path::Path::new("/etc/systemd/system").join(unit_name);

            // Write existing content (or original) as starting point.
            if !etc_path.exists()
                && !original.is_empty()
                && let Err(e) = std::fs::write(&etc_path, original)
            {
                if !quiet {
                    eprintln!("Failed to write {}: {e}", etc_path.display());
                }
                std::process::exit(1);
            }

            let status = std::process::Command::new(&editor).arg(&etc_path).status();
            match status {
                Ok(s) if s.success() => {}
                Ok(s) => {
                    if !quiet {
                        eprintln!("Editor exited with status {}", s.code().unwrap_or(-1));
                    }
                    std::process::exit(1);
                }
                Err(e) => {
                    if !quiet {
                        eprintln!("Failed to run editor '{}': {e}", editor);
                    }
                    std::process::exit(1);
                }
            }
        } else {
            // Drop-in mode: edit /etc/systemd/system/<unit>.d/override.conf.
            let default_dropin_dir = format!("/etc/systemd/system/{unit_name}.d");
            let dropin_dir = info
                .get("dropin_dir")
                .and_then(|v| v.as_str())
                .unwrap_or(&default_dropin_dir);
            let default_override_path = format!("/etc/systemd/system/{unit_name}.d/override.conf");
            let override_path = info
                .get("override_path")
                .and_then(|v| v.as_str())
                .unwrap_or(&default_override_path);
            let existing = info
                .get("existing_override")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Ensure the drop-in directory exists.
            if let Err(e) = std::fs::create_dir_all(dropin_dir) {
                if !quiet {
                    eprintln!("Failed to create {dropin_dir}: {e}");
                }
                std::process::exit(1);
            }

            // Write a temp file with existing content for the editor.
            let tmp_path = format!("{override_path}.tmp");
            let initial_content = if existing.is_empty() {
                "### Editing drop-in override for {}\n### Anything between here and the comment below will become the contents of the drop-in file\n\n[Service]\n\n### Lines below this comment will be discarded\n".replace("{}", unit_name)
            } else {
                existing.to_owned()
            };
            if let Err(e) = std::fs::write(&tmp_path, &initial_content) {
                if !quiet {
                    eprintln!("Failed to write {tmp_path}: {e}");
                }
                std::process::exit(1);
            }

            let status = std::process::Command::new(&editor).arg(&tmp_path).status();

            match status {
                Ok(s) if s.success() => {
                    // Read the edited content and write to the override path.
                    match std::fs::read_to_string(&tmp_path) {
                        Ok(edited) => {
                            // Strip comment lines starting with ### for the template.
                            let clean: String = edited
                                .lines()
                                .filter(|l| !l.starts_with("### "))
                                .collect::<Vec<_>>()
                                .join("\n");
                            let trimmed = clean.trim();
                            if trimmed.is_empty() {
                                // Empty content — remove the override if it existed.
                                let _ = std::fs::remove_file(override_path);
                                if !quiet {
                                    eprintln!("Removed empty override for {unit_name}.");
                                }
                            } else {
                                let mut final_content = trimmed.to_owned();
                                if !final_content.ends_with('\n') {
                                    final_content.push('\n');
                                }
                                if let Err(e) = std::fs::write(override_path, &final_content) {
                                    if !quiet {
                                        eprintln!("Failed to write {override_path}: {e}");
                                    }
                                    std::process::exit(1);
                                }
                            }
                        }
                        Err(e) => {
                            if !quiet {
                                eprintln!("Failed to read {tmp_path}: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                    // Clean up temp file.
                    let _ = std::fs::remove_file(&tmp_path);
                }
                Ok(s) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    if !quiet {
                        eprintln!("Editor exited with status {}", s.code().unwrap_or(-1));
                    }
                    std::process::exit(1);
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    if !quiet {
                        eprintln!("Failed to run editor '{}': {e}", editor);
                    }
                    std::process::exit(1);
                }
            }
        }

        // Trigger daemon-reload after editing.
        let reload_call = Call {
            method: "reload".to_string(),
            params: None,
            id: None,
        };
        let reload_str = serde_json::to_string(&reload_call.to_json()).unwrap();
        let _ = if addr.starts_with('/') {
            send_unix(&addr, &reload_str)
        } else {
            send_tcp(&addr, &reload_str)
        };
        return;
    }

    // Handle get-default/set-default locally (filesystem operations).
    if positional[0] == "get-default" {
        let root = root_path.as_deref().unwrap_or("");
        let link_path = format!("{root}/etc/systemd/system/default.target");
        match std::fs::read_link(&link_path) {
            Ok(target) => {
                let name = target
                    .file_name()
                    .unwrap_or(target.as_os_str())
                    .to_string_lossy();
                println!("{name}");
            }
            Err(_) => {
                // Fallback: check /usr/lib/systemd/system/default.target
                let fallback = format!("{root}/usr/lib/systemd/system/default.target");
                match std::fs::read_link(&fallback) {
                    Ok(target) => {
                        let name = target
                            .file_name()
                            .unwrap_or(target.as_os_str())
                            .to_string_lossy();
                        println!("{name}");
                    }
                    Err(_) => {
                        // Default to multi-user.target if no symlink exists
                        println!("multi-user.target");
                    }
                }
            }
        }
        return;
    }

    if positional[0] == "set-default" {
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: set-default requires a target unit name.");
            }
            std::process::exit(1);
        }
        let target_name = &positional[1];
        let root = root_path.as_deref().unwrap_or("");
        let dir_path = format!("{root}/etc/systemd/system");
        let link_path = format!("{dir_path}/default.target");

        // Ensure the directory exists
        if let Err(e) = std::fs::create_dir_all(&dir_path) {
            if !quiet {
                eprintln!("Failed to create directory {dir_path}: {e}");
            }
            std::process::exit(1);
        }

        // Remove existing symlink
        let _ = std::fs::remove_file(&link_path);

        // Resolve to full path if not absolute
        let target_path = if target_name.contains('/') {
            target_name.to_string()
        } else {
            format!("/usr/lib/systemd/system/{target_name}")
        };

        if let Err(e) = std::os::unix::fs::symlink(&target_path, &link_path) {
            if !quiet {
                eprintln!("Failed to create symlink {link_path} -> {target_path}: {e}");
            }
            std::process::exit(1);
        }

        if !quiet {
            eprintln!("Created symlink {link_path} -> {target_path}.");
        }
        return;
    }

    // --dry-run: print what would happen and exit
    if dry_run {
        let verb = command;
        if !quiet {
            eprintln!("Would execute operation: {verb}");
        }
        return;
    }

    let method = command.clone();
    let params = if method == "list-units" {
        // list-units [type] — optional type or state filter
        // Build a JSON object with optional type and state filters.
        let mut obj = serde_json::Map::new();
        if positional.len() >= 2 {
            obj.insert("type".to_string(), Value::String(positional[1].clone()));
        }
        if let Some(ref state) = state_filter {
            obj.insert("state".to_string(), Value::String(state.clone()));
        }
        if obj.is_empty() {
            None
        } else {
            Some(Value::Object(obj))
        }
    } else if method == "list-timers"
        || method == "list-sockets"
        || method == "list-paths"
        || method == "list-jobs"
        || method == "is-system-running"
        || method == "show-environment"
    {
        // These commands take no parameters
        None
    } else if method == "set-environment" || method == "unset-environment" {
        // set-environment KEY=VALUE... / unset-environment KEY...
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: {method} requires at least one argument.");
            }
            std::process::exit(1);
        }
        let arr: Vec<Value> = positional[1..].iter().cloned().map(Value::String).collect();
        Some(Value::Array(arr))
    } else if method == "import-environment" {
        // import-environment KEY... — send KEY=VALUE pairs from our own env
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: import-environment requires at least one variable name.");
            }
            std::process::exit(1);
        }
        let arr: Vec<Value> = positional[1..]
            .iter()
            .map(|name| {
                if let Ok(val) = std::env::var(name) {
                    Value::String(format!("{name}={val}"))
                } else {
                    Value::String(name.clone())
                }
            })
            .collect();
        Some(Value::Array(arr))
    } else if method == "set-property" {
        // set-property <unit> <prop=val>...
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: set-property requires a unit name.");
            }
            std::process::exit(1);
        }
        if positional.len() < 3 {
            if !quiet {
                eprintln!(
                    "Error: set-property requires at least one property assignment (e.g. CPUWeight=200)."
                );
            }
            std::process::exit(1);
        }
        let mut arr: Vec<Value> = positional[1..].iter().cloned().map(Value::String).collect();
        if runtime {
            arr.push(Value::String("--runtime".to_string()));
        }
        Some(Value::Array(arr))
    } else if method == "revert" {
        // revert <unit>
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: revert requires a unit name.");
            }
            std::process::exit(1);
        }
        Some(Value::String(positional[1].clone()))
    } else if method == "list-unit-files" {
        // list-unit-files [--type=TYPE] — optional type filter extracted from -t flag
        // Check if there's a type filter passed as a positional argument
        if positional.len() >= 2 {
            Some(Value::String(positional[1].clone()))
        } else {
            None
        }
    } else if method == "list-dependencies" {
        // list-dependencies <unit> [--reverse]
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: list-dependencies requires a unit name.");
            }
            std::process::exit(1);
        }
        let mut arr = vec![Value::String(positional[1].clone())];
        if reverse {
            arr.push(Value::String("--reverse".to_owned()));
        }
        if after {
            arr.push(Value::String("--after".to_owned()));
        }
        if before {
            arr.push(Value::String("--before".to_owned()));
        }
        if plain {
            arr.push(Value::String("--plain".to_owned()));
        }
        Some(Value::Array(arr))
    } else if method == "kill" {
        // kill <unit> [--signal=SIG] [--kill-whom=WHO] [--kill-value=N]
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: kill requires a unit name.");
            }
            std::process::exit(1);
        }
        let mut arr = vec![Value::String(positional[1].clone())];
        if let Some(sig) = kill_signal {
            arr.push(Value::String(sig.to_string()));
        } else {
            arr.push(Value::String("15".to_string())); // SIGTERM default
        }
        arr.push(Value::String(
            kill_whom.unwrap_or_else(|| "all".to_string()),
        ));
        if let Some(val) = kill_value {
            arr.push(Value::String(val.to_string()));
        }
        if wait {
            arr.push(Value::String("--wait".to_string()));
        }
        Some(Value::Array(arr))
    } else if method == "suspend"
        || method == "hibernate"
        || method == "hybrid-sleep"
        || method == "suspend-then-hibernate"
    {
        // Sleep commands take no parameters.
        None
    } else if method == "enable" || method == "reenable" {
        // enable <unit>... [--runtime] [--root=PATH]
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: enable requires at least one unit name.");
            }
            std::process::exit(1);
        }
        let mut arr: Vec<Value> = positional[1..].iter().cloned().map(Value::String).collect();
        if runtime {
            arr.push(Value::String("--runtime".to_string()));
        }
        if let Some(ref root) = root_path {
            arr.push(Value::String(format!("--root={root}")));
        }
        Some(Value::Array(arr))
    } else if method == "disable" {
        // disable <unit>... [--runtime] [--root=PATH]
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: disable requires at least one unit name.");
            }
            std::process::exit(1);
        }
        let mut arr: Vec<Value> = positional[1..].iter().cloned().map(Value::String).collect();
        if runtime {
            arr.push(Value::String("--runtime".to_string()));
        }
        if let Some(ref root) = root_path {
            arr.push(Value::String(format!("--root={root}")));
        }
        Some(Value::Array(arr))
    } else if method == "preset" {
        // preset <unit>... [--preset-mode=MODE] [--runtime] [--root=PATH]
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: preset requires at least one unit name.");
            }
            std::process::exit(1);
        }
        let mut arr: Vec<Value> = positional[1..].iter().cloned().map(Value::String).collect();
        if runtime {
            arr.push(Value::String("--runtime".to_string()));
        }
        if let Some(ref mode) = preset_mode {
            arr.push(Value::String(format!("--preset-mode={mode}")));
        }
        if let Some(ref root) = root_path {
            arr.push(Value::String(format!("--root={root}")));
        }
        Some(Value::Array(arr))
    } else if method == "reset-failed" {
        // reset-failed [unit] — optional unit name
        if positional.len() >= 2 {
            Some(Value::String(positional[1].clone()))
        } else {
            None
        }
    } else if method == "clean" {
        // clean <unit> [--what=WHAT]
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: clean requires a unit name.");
            }
            std::process::exit(1);
        }
        let mut arr = vec![Value::String(positional[1].clone())];
        if let Some(ref what) = what_filter {
            arr.push(Value::String(what.clone()));
        }
        Some(Value::Array(arr))
    } else if method == "show" {
        // show [unit] [property-filter...] — send unit name + optional filter
        // If no unit is specified, query manager-level properties
        if positional.len() < 2 {
            // Manager-level show — send special "__manager__" marker
            let mut arr = vec![Value::String("__manager__".to_string())];
            for prop in &property_filter {
                arr.push(Value::String(prop.clone()));
            }
            Some(Value::Array(arr))
        } else {
            let mut arr = vec![Value::String(positional[1].clone())];
            for prop in &property_filter {
                arr.push(Value::String(prop.clone()));
            }
            Some(Value::Array(arr))
        }
    } else if method == "mask" || method == "unmask" {
        // mask/unmask <unit>... [--runtime]
        if positional.len() < 2 {
            if !quiet {
                eprintln!("Error: {method} requires at least one unit name.");
            }
            std::process::exit(1);
        }
        let mut arr: Vec<Value> = positional[1..].iter().cloned().map(Value::String).collect();
        if runtime {
            arr.push(Value::String("--runtime".to_string()));
        }
        if let Some(ref root) = root_path {
            arr.push(Value::String(format!("--root={root}")));
        }
        Some(Value::Array(arr))
    } else if marked && (method == "reload-or-restart" || method == "restart") {
        // --marked: operate on all units with needs-restart marker
        Some(Value::String("--marked".to_string()))
    } else if root_path.is_some()
        && (method == "is-enabled" || method == "list-unit-files" || method == "revert")
    {
        // Commands that need --root passed to the server
        let mut arr: Vec<Value> = positional[1..].iter().cloned().map(Value::String).collect();
        if let Some(ref root) = root_path {
            arr.push(Value::String(format!("--root={root}")));
        }
        Some(Value::Array(arr))
    } else if positional.len() == 2 {
        Some(Value::String(positional[1].clone()))
    } else if positional.len() > 2 {
        Some(positional[1..].iter().cloned().map(Value::String).collect())
    } else {
        None
    };

    // --no-block: use a separate method so the server can run it asynchronously
    let method = if no_block && (method == "start" || method == "restart" || method == "stop") {
        format!("{method}-noblock")
    } else {
        method.to_string()
    };
    let method_name = method.clone();
    let call = Call {
        method,
        params,
        id: None,
    };
    let str_call = serde_json::to_string(&call.to_json()).unwrap();

    // Retry on connection refused (e.g. after daemon-reexec, the socket
    // may not be ready yet).
    let mut result = if addr.starts_with('/') {
        send_unix(&addr, &str_call)
    } else {
        send_tcp(&addr, &str_call)
    };
    // daemon-reexec causes the server to execve(), dropping the connection.
    // Don't retry — the broken connection IS the success signal.
    let is_daemon_reexec = positional[0] == "daemon-reexec";
    if result.is_err() && !is_daemon_reexec {
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            result = if addr.starts_with('/') {
                send_unix(&addr, &str_call)
            } else {
                send_tcp(&addr, &str_call)
            };
            if result.is_ok() {
                break;
            }
        }
    }

    match result {
        Ok(resp) => {
            handle_response(
                &positional[0],
                &resp,
                quiet,
                value_only,
                force,
                output_format.as_deref(),
                &property_filter,
            );
            // --now: after enable → start units, after disable → stop units
            if now
                && (method_name == "enable"
                    || method_name == "reenable"
                    || method_name == "disable")
            {
                let action = if method_name == "disable" {
                    "stop"
                } else {
                    "start"
                };
                // For template units (e.g. foo@.service), find and act on all instances
                let mut units_to_act_on: Vec<String> = Vec::new();
                for unit in &positional[1..] {
                    if unit.contains("@.") {
                        // Template unit — query for active instances by listing units
                        let prefix = unit.split("@.").next().unwrap_or("");
                        let list_call = serde_json::json!({
                            "method": "list-units",
                        });
                        if let Ok(list_resp) = send_unix(&addr, &list_call.to_string())
                            && let Some(result) = list_resp.get("result")
                            && let Some(arr) = result.as_array()
                        {
                            for entry in arr {
                                // list-units returns plain strings or objects with "UNIT" key
                                let name = entry
                                    .as_str()
                                    .or_else(|| entry.get("UNIT").and_then(|v| v.as_str()));
                                if let Some(name) = name
                                    && name.starts_with(&format!("{prefix}@"))
                                    && !name.contains("@.")
                                {
                                    units_to_act_on.push(name.to_string());
                                }
                            }
                        }
                    } else {
                        units_to_act_on.push(unit.clone());
                    }
                }
                for unit in &units_to_act_on {
                    let call = serde_json::json!({
                        "method": action,
                        "params": unit,
                    });
                    let _ = send_unix(&addr, &call.to_string());
                }
            }
        }
        Err(e) => {
            // daemon-reexec causes the server to execve(), dropping the connection.
            // This is expected — treat it as success.
            if positional[0] == "daemon-reexec" {
                return;
            }
            if !quiet {
                eprintln!("Error communicating with rust-systemd: {e}");
            }
            // For is-active, connection failure means the unit is not active.
            if positional[0] == "is-active" {
                if !quiet {
                    println!("inactive");
                }
                std::process::exit(3);
            }
            // For is-system-running, connection failure means offline.
            if positional[0] == "is-system-running" {
                if !quiet {
                    println!("offline");
                }
                std::process::exit(1);
            }
            std::process::exit(1);
        }
    }
}

/// Handle the JSON-RPC response, with special exit code logic for
/// `is-active`, `is-enabled`, and `is-failed`.
fn handle_response(
    command: &str,
    resp: &Value,
    quiet: bool,
    value_only: bool,
    force: bool,
    output_format: Option<&str>,
    property_filter: &[String],
) {
    // Check for JSON-RPC error responses.
    if let Some(error) = resp.get("error") {
        let message = error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");

        match command {
            "is-active" => {
                // Exit 4 for "not found", 3 for other errors (inactive)
                let code = if message.contains("not found") { 4 } else { 3 };
                if !quiet {
                    println!("inactive");
                }
                std::process::exit(code);
            }
            "is-enabled" => {
                // Exit 4 for "not found", 1 for other errors
                let code = if message.contains("not found") { 4 } else { 1 };
                if !quiet {
                    println!("disabled");
                }
                std::process::exit(code);
            }
            "is-failed" => {
                // Exit 4 for "not found", 1 for other errors
                let code = if message.contains("not found") { 4 } else { 1 };
                if !quiet {
                    println!("inactive");
                }
                std::process::exit(code);
            }
            "cat" if force => {
                // --force suppresses errors for missing units
                std::process::exit(0);
            }
            _ => {
                if !quiet {
                    eprintln!("{}", message);
                }
                std::process::exit(1);
            }
        }
    }

    // Success response.
    let result = resp.get("result");

    match command {
        "is-active" => {
            let state = result.and_then(|v| v.as_str()).unwrap_or("active");
            if !quiet {
                println!("{}", state);
            }
            match state {
                "active" | "reloading" => std::process::exit(0),
                _ => std::process::exit(3),
            }
        }
        "is-enabled" => {
            let state = result.and_then(|v| v.as_str()).unwrap_or("enabled");
            if !quiet {
                println!("{}", state);
            }
            match state {
                "enabled" | "enabled-runtime" | "static" | "indirect" | "generated" => {
                    std::process::exit(0)
                }
                _ => std::process::exit(1),
            }
        }
        "is-failed" => {
            let state = result.and_then(|v| v.as_str()).unwrap_or("inactive");
            if !quiet {
                println!("{}", state);
            }
            match state {
                "failed" => std::process::exit(0),
                _ => std::process::exit(1),
            }
        }
        "show-environment" => {
            if let Some(result) = result
                && let Some(env_lines) = result.get("environment").and_then(|v| v.as_array())
                && !quiet
            {
                if output_format == Some("json") {
                    // JSON output: {"KEY":"VALUE", ...}
                    let mut obj = serde_json::Map::new();
                    for line in env_lines {
                        if let Some(s) = line.as_str()
                            && let Some((k, v)) = s.split_once('=')
                        {
                            obj.insert(k.to_owned(), Value::String(v.to_owned()));
                        }
                    }
                    println!("{}", serde_json::to_string(&obj).unwrap_or_default());
                } else {
                    for line in env_lines {
                        if let Some(s) = line.as_str() {
                            println!("{s}");
                        }
                    }
                }
            }
        }
        "set-environment" | "unset-environment" | "import-environment" => {
            // No output needed on success
        }
        "is-system-running" => {
            let state = result.and_then(|v| v.as_str()).unwrap_or("offline");
            if !quiet {
                println!("{}", state);
            }
            match state {
                "running" | "initializing" | "starting" => std::process::exit(0),
                _ => std::process::exit(1),
            }
        }
        "show" => {
            // The result contains { "show": "Key=Value\n..." }
            if let Some(result) = result
                && let Some(text) = result.get("show").and_then(|v| v.as_str())
                && !quiet
            {
                if value_only && !property_filter.is_empty() {
                    // --value mode: print only the values, one per line
                    for line in text.lines() {
                        if let Some((_key, val)) = line.split_once('=') {
                            println!("{val}");
                        }
                    }
                } else {
                    print!("{text}");
                }
            }
        }
        "cat" => {
            // The result contains { "cat": "# /path/to/unit\n[Unit]\n..." }
            if let Some(result) = result
                && let Some(text) = result.get("cat").and_then(|v| v.as_str())
                && !quiet
            {
                print!("{text}");
            }
        }
        "list-unit-files" => {
            if let Some(result) = result
                && let Some(text) = result.get("list-unit-files").and_then(|v| v.as_str())
                && !quiet
            {
                print!("{text}");
            }
        }
        "enable" | "reenable" => {
            if let Some(result) = result
                && let Some(arr) = result.get("enabled").and_then(|v| v.as_array())
                && !quiet
            {
                for name in arr {
                    if let Some(s) = name.as_str() {
                        println!("Created symlink for {s}.");
                    }
                }
            }
        }
        "disable" => {
            if let Some(result) = result
                && let Some(arr) = result.get("disabled").and_then(|v| v.as_array())
                && !quiet
            {
                for name in arr {
                    if let Some(s) = name.as_str() {
                        println!("Removed /etc/systemd/system/multi-user.target.wants/{s}.");
                    }
                }
            }
        }
        "reset-failed"
        | "kill"
        | "suspend"
        | "hibernate"
        | "hybrid-sleep"
        | "suspend-then-hibernate"
        | "shutdown"
        | "poweroff"
        | "reboot"
        | "halt" => {
            // These return null on success — nothing to print.
        }
        "set-property" => {
            if let Some(result) = result
                && !quiet
                && let Some(dropin) = result.get("dropin").and_then(|v| v.as_str())
            {
                println!("Created drop-in file: {dropin}");
            }
        }
        "revert" => {
            if let Some(result) = result
                && !quiet
                && let Some(unit) = result.get("reverted").and_then(|v| v.as_str())
                && let Some(arr) = result.get("removed").and_then(|v| v.as_array())
            {
                if arr.is_empty() {
                    println!("No overrides found for {unit}.");
                } else {
                    for path in arr {
                        if let Some(s) = path.as_str() {
                            println!("Removed {s}.");
                        }
                    }
                }
            }
        }
        "list-timers" => {
            if let Some(result) = result
                && let Some(arr) = result.as_array()
                && !quiet
            {
                format_timer_table(arr);
            }
        }
        "list-sockets" => {
            if let Some(result) = result
                && let Some(arr) = result.as_array()
                && !quiet
            {
                if arr.is_empty() {
                    println!("0 sockets listed.");
                } else {
                    println!("{:<40} {:<8} {:<40}", "LISTEN", "TYPE", "UNIT");
                    for socket in arr {
                        let listen = socket.get("LISTEN").and_then(|v| v.as_str()).unwrap_or("");
                        let stype = socket.get("TYPE").and_then(|v| v.as_str()).unwrap_or("");
                        let unit = socket.get("UNIT").and_then(|v| v.as_str()).unwrap_or("");
                        println!("{:<40} {:<8} {:<40}", listen, stype, unit);
                    }
                    println!("\n{} sockets listed.", arr.len());
                }
            }
        }
        "list-paths" => {
            if let Some(result) = result
                && let Some(arr) = result.as_array()
                && !quiet
            {
                if arr.is_empty() {
                    println!("0 paths listed.");
                } else {
                    for path_entry in arr {
                        let path = path_entry
                            .get("PATH")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let unit = path_entry
                            .get("UNIT")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        println!("{:<40} {:<40}", path, unit);
                    }
                    println!("\n{} paths listed.", arr.len());
                }
            }
        }
        "list-jobs" => {
            if let Some(result) = result
                && let Some(arr) = result.as_array()
            {
                for job in arr {
                    let id = job.get("JOB").and_then(|v| v.as_u64()).unwrap_or(0);
                    let unit = job
                        .get("UNIT")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let jtype = job.get("TYPE").and_then(|v| v.as_str()).unwrap_or("start");
                    let state = job
                        .get("STATE")
                        .and_then(|v| v.as_str())
                        .unwrap_or("running");
                    println!("{id:>6} {unit:<48} {jtype:<15} {state}");
                }
                println!("\n{} jobs listed.", arr.len());
            }
        }
        "list-units" => {
            // list-units returns an array of unit name strings.
            if let Some(result) = result
                && let Some(arr) = result.as_array()
                && !quiet
            {
                for name in arr {
                    if let Some(s) = name.as_str() {
                        println!("{s}");
                    }
                }
            }
        }
        "list-dependencies" => {
            if let Some(result) = result
                && let Some(text) = result.get("list-dependencies").and_then(|v| v.as_str())
                && !quiet
            {
                print!("{text}");
            }
        }
        "service-watchdogs" | "log-level" | "log-target" => {
            // Manager property get/set: print raw string value
            if let Some(result) = result
                && !quiet
                && let Some(s) = result.as_str()
            {
                println!("{s}");
            }
        }
        "help" => {
            // systemctl help <unit> — success, nothing to print
        }
        "mask" => {
            if let Some(result) = result
                && let Some(arr) = result.get("masked").and_then(|v| v.as_array())
                && !quiet
            {
                for name in arr {
                    if let Some(s) = name.as_str() {
                        println!("Created symlink /etc/systemd/system/{s} → /dev/null.");
                    }
                }
            }
        }
        "unmask" => {
            if let Some(result) = result
                && let Some(arr) = result.get("unmasked").and_then(|v| v.as_array())
                && !quiet
            {
                for name in arr {
                    if let Some(s) = name.as_str() {
                        println!("Removed /etc/systemd/system/{s}.");
                    }
                }
            }
        }
        _ => {
            // For all other commands, print the result if non-null and non-empty.
            if !quiet && let Some(result) = result {
                let is_empty = result.is_null() || result.as_array().is_some_and(|a| a.is_empty());
                if !is_empty {
                    println!("{}", serde_json::to_string_pretty(result).unwrap());
                }
            }
        }
    }
}

fn format_timer_table(timers: &[Value]) {
    if timers.is_empty() {
        println!("0 timers listed.");
        return;
    }
    // Print header
    println!(
        "{:<40} {:<8} {:<40} TRIGGERS",
        "UNIT", "ACTIVE", "ACTIVATES"
    );
    for timer in timers {
        let unit = timer.get("UNIT").and_then(|v| v.as_str()).unwrap_or("");
        let active = timer.get("ACTIVE").and_then(|v| v.as_str()).unwrap_or("");
        let activates = timer
            .get("ACTIVATES")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let triggers = timer.get("TRIGGERS").and_then(|v| v.as_str()).unwrap_or("");
        println!("{:<40} {:<8} {:<40} {}", unit, active, activates, triggers);
    }
    println!("\n{} timers listed.", timers.len());
}

fn print_help() {
    println!(
        "\
systemctl — control tool for the rust-systemd service manager

Usage:
    systemctl [OPTIONS] <command> [args...]

The control socket defaults to /run/systemd/rust-systemd-notify/control.socket.
You can also set the SYSTEMCTL_ADDR environment variable, or pass a socket
path or TCP address as the first positional argument.

Commands:
    list-units                  List all loaded units
    list-unit-files [TYPE]      List all unit files on disk with their state
    list-dependencies <unit>    Show dependency tree for a unit
    disable <unit>...           Disable one or more units
    reset-failed [unit]         Reset the failed state of a unit (or all)
    kill <unit>                 Send a signal to a unit's processes
    suspend                     Suspend the system
    hibernate                   Hibernate the system
    hybrid-sleep                Hybrid suspend (RAM + disk)
    suspend-then-hibernate      Suspend, then hibernate after a delay
    status <unit>               Show status of a unit
    show <unit>                 Show properties of a unit (key=value format)
    cat <unit>                  Show the unit file source
    edit <unit>                 Edit a unit file drop-in override (or --full for full copy)
    set-property <unit> <P=V>   Set runtime properties on a unit (creates drop-in)
    revert <unit>               Revert a unit to its vendor configuration
    start <unit>                Start a unit
    stop <unit>                 Stop a unit
    restart <unit>              Restart a unit
    try-restart <unit>          Restart a unit if it is active
    reload-or-restart <unit>    Reload or restart a unit
    is-active <unit>            Check if a unit is active (exit 0=yes, 3=no)
    is-enabled <unit>           Check if a unit is enabled
    is-failed <unit>            Check if a unit is in failed state
    enable <unit>               Enable (load) a unit
    mask <unit>...              Mask (symlink to /dev/null) one or more units
    unmask <unit>...            Unmask (remove /dev/null symlinks) one or more units
    daemon-reload               Reload the service manager configuration
    poweroff                    Power off the system
    reboot                      Reboot the system
    halt                        Halt the system
    shutdown                    Shut down the service manager

Options:
    --no-block                  Do not wait for the operation to complete
    --quiet, -q                 Suppress output
    --no-wall                   Do not send wall message before shutdown
    --force, -f                 Force the operation
    --no-pager                  Do not pipe output into a pager
    --no-ask-password           Do not ask for password
    --no-legend                 Do not print legend (column headers)
    --system                    Connect to system manager (default)
    --full                      Edit full unit file instead of drop-in (for edit)
    --all, -a                   Show all units, including inactive
    --reverse                   Show reverse dependencies (for list-dependencies)
    -s, --signal <SIG>          Signal to send (for kill, default: SIGTERM)
    -t, --type <TYPE>           Filter by unit type
    -p, --property <PROP>       Show only specified property (for show)
    --value                     Show only property values (with -p)
    --help, -h                  Show this help
    --version                   Show version

Examples:
    systemctl list-units
    systemctl status sshd.service
    systemctl show sshd.service
    systemctl show -p MainPID,ActiveState sshd.service
    systemctl show --value -p MainPID sshd.service
    systemctl cat sshd.service
    systemctl edit sshd.service
    systemctl edit --full sshd.service
    systemctl set-property sshd.service CPUWeight=200
    systemctl revert sshd.service
    systemctl restart nginx.service
    systemctl --no-block try-restart nscd.service
    systemctl is-active sshd.service
    systemctl list-unit-files
    systemctl list-unit-files service
    systemctl disable tmp.mount
    systemctl reset-failed sshd.service
    systemctl kill --signal=SIGKILL sshd.service
    systemctl suspend
    systemctl hibernate
    systemctl list-dependencies multi-user.target
    systemctl list-dependencies --reverse sshd.service
    systemctl mask tmp.mount
    systemctl unmask tmp.mount"
    );
}

fn send_unix(path: &str, payload: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let mut stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.write_all(payload.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let resp: Value = serde_json::from_reader(&mut stream)?;
    Ok(resp)
}

fn send_tcp(addr: &str, payload: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let mut stream = std::net::TcpStream::connect(addr)?;
    stream.write_all(payload.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let resp: Value = serde_json::from_reader(&mut stream)?;
    Ok(resp)
}
