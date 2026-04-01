//! loginctl — Control the systemd login manager.
//!
//! This tool communicates with systemd-logind via its control socket
//! to query and manage sessions, seats, users, and inhibitor locks.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process;

const CONTROL_SOCKET_PATH: &str = "/run/systemd/logind-control";

// ---------------------------------------------------------------------------
// CLI parsing (manual, to avoid heavy deps and match systemd style)
// ---------------------------------------------------------------------------

fn usage() {
    eprintln!(
        r#"loginctl — Control the systemd login manager

Session Commands:
  list-sessions                List current sessions
  session-status [ID]          Show session status
  show-session [ID]            Show properties of a session
  activate [ID]                Activate a session
  lock-session [ID]            Lock a session
  unlock-session [ID]          Unlock a session
  terminate-session [ID]       Terminate a session
  kill-session [ID]            Send signal to session processes

User Commands:
  list-users                   List logged in users
  user-status [USER]           Show user status
  show-user [USER]             Show properties of a user
  enable-linger [USER]         Enable lingering for a user
  disable-linger [USER]        Disable lingering for a user
  terminate-user [USER]        Terminate all sessions of a user
  kill-user [USER]             Send signal to user's processes

Seat Commands:
  list-seats                   List available seats
  seat-status [NAME]           Show seat status
  show-seat [NAME]             Show properties of a seat

System Commands:
  poweroff                     Power off the machine
  reboot                       Reboot the machine
  suspend                      Suspend the machine
  hibernate                    Hibernate the machine

Options:
  -h, --help                   Show this help
  -a, --all                    Show all properties
  --no-pager                   Do not pipe output into a pager
  --no-legend                  Do not show column headers
  --no-ask-password            Do not ask for system passwords
  -H, --host HOST              Operate on remote host
  -M, --machine CONTAINER      Operate on local container
  -p, --property PROP          Show only specified properties
  -l, --full                   Do not ellipsize output"#
    );
}

fn send_command(cmd: &str) -> Result<String, String> {
    let mut stream = UnixStream::connect(CONTROL_SOCKET_PATH).map_err(|e| {
        format!(
            "Failed to connect to logind control socket at {}: {}",
            CONTROL_SOCKET_PATH, e
        )
    })?;

    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .map_err(|e| format!("Failed to set read timeout: {}", e))?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|e| format!("Failed to set write timeout: {}", e))?;

    stream
        .write_all(cmd.as_bytes())
        .map_err(|e| format!("Failed to send command: {}", e))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("Failed to shutdown write: {}", e))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| format!("Failed to read response: {}", e))?;

    Ok(response)
}

// ---------------------------------------------------------------------------
// Session formatting
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Session {
    id: String,
    uid: u32,
    user: String,
    seat: Option<String>,
    tty: String,
    state: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    vtnr: u32,
    #[serde(default)]
    session_type: String,
    #[serde(default)]
    class: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    leader: u32,
    #[serde(default)]
    since: u64,
}

#[derive(serde::Deserialize)]
struct Seat {
    id: String,
    active_session: Option<String>,
    sessions: Vec<String>,
    #[serde(default)]
    can_graphical: bool,
    #[serde(default)]
    can_multi_session: bool,
}

#[derive(serde::Deserialize)]
struct User {
    uid: u32,
    name: String,
    state: String,
    sessions: Vec<String>,
    #[serde(default)]
    slice: String,
    #[serde(default)]
    since: u64,
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct Inhibitor {
    what: String,
    who: String,
    why: String,
    mode: String,
    uid: u32,
    pid: u32,
    #[serde(default)]
    id: u64,
    #[serde(default)]
    since: u64,
}

fn print_sessions(json: &str, no_legend: bool) {
    let sessions: Vec<Session> = match serde_json::from_str(json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to parse sessions: {}", e);
            return;
        }
    };

    if !no_legend {
        println!(
            "{:<10} {:<10} {:<16} {:<12} {:<8}",
            "SESSION", "UID", "USER", "SEAT", "TTY"
        );
    }

    for s in &sessions {
        println!(
            "{:<10} {:<10} {:<16} {:<12} {:<8}",
            s.id,
            s.uid,
            s.user,
            s.seat.as_deref().unwrap_or("-"),
            if s.tty.is_empty() { "-" } else { &s.tty }
        );
    }

    if !no_legend {
        println!("\n{} sessions listed.", sessions.len());
    }
}

fn print_seats(json: &str, no_legend: bool) {
    let seats: Vec<Seat> = match serde_json::from_str(json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to parse seats: {}", e);
            return;
        }
    };

    if !no_legend {
        println!("{:<16}", "SEAT");
    }

    for s in &seats {
        println!("{:<16}", s.id);
    }

    if !no_legend {
        println!("\n{} seats listed.", seats.len());
    }
}

fn print_users(json: &str, no_legend: bool) {
    let users: Vec<User> = match serde_json::from_str(json) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Failed to parse users: {}", e);
            return;
        }
    };

    if !no_legend {
        println!("{:<10} {:<16}", "UID", "USER");
    }

    for u in &users {
        println!("{:<10} {:<16}", u.uid, u.name);
    }

    if !no_legend {
        println!("\n{} users listed.", users.len());
    }
}

fn print_inhibitors(json: &str, no_legend: bool) {
    let inhibitors: Vec<Inhibitor> = match serde_json::from_str(json) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to parse inhibitors: {}", e);
            return;
        }
    };

    if !no_legend {
        println!(
            "{:<24} {:<24} {:<24} {:<8} {:<8} {:<8}",
            "WHAT", "WHO", "WHY", "MODE", "UID", "PID"
        );
    }

    for i in &inhibitors {
        println!(
            "{:<24} {:<24} {:<24} {:<8} {:<8} {:<8}",
            i.what, i.who, i.why, i.mode, i.uid, i.pid
        );
    }

    if !no_legend {
        println!("\n{} inhibitors listed.", inhibitors.len());
    }
}

fn print_session_status(json: &str) {
    let session: Session = match serde_json::from_str(json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to parse session: {}", e);
            return;
        }
    };

    println!("{} - {}", session.id, session.user);
    println!(
        "           Since: {}",
        if session.since > 0 {
            format_timestamp(session.since)
        } else {
            "-".to_string()
        }
    );
    println!(
        "          Leader: {}",
        if session.leader > 0 {
            session.leader.to_string()
        } else {
            "-".to_string()
        }
    );
    println!(
        "            Seat: {}",
        session.seat.as_deref().unwrap_or("-")
    );
    if session.vtnr > 0 {
        println!("             TTY: {} (VT {})", session.tty, session.vtnr);
    } else if !session.tty.is_empty() {
        println!("             TTY: {}", session.tty);
    }
    println!("            Type: {}", session.session_type);
    println!("           Class: {}", session.class);
    println!("           Scope: {}", session.scope);
    println!(
        "          Active: {}",
        if session.active { "yes" } else { "no" }
    );
    println!("           State: {}", session.state);
    println!("             UID: {}", session.uid);
}

fn print_seat_status(json: &str) {
    let seat: Seat = match serde_json::from_str(json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to parse seat: {}", e);
            return;
        }
    };

    println!("{}", seat.id);
    println!(
        "       Sessions: {}",
        if seat.sessions.is_empty() {
            "(none)".to_string()
        } else {
            seat.sessions.join(" ")
        }
    );
    println!(
        "  ActiveSession: {}",
        seat.active_session.as_deref().unwrap_or("(none)")
    );
    println!(
        "   CanGraphical: {}",
        if seat.can_graphical { "yes" } else { "no" }
    );
    println!(
        "CanMultiSession: {}",
        if seat.can_multi_session { "yes" } else { "no" }
    );
}

fn print_user_status(json: &str) {
    let user: User = match serde_json::from_str(json) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Failed to parse user: {}", e);
            return;
        }
    };

    println!("{} ({})", user.name, user.uid);
    println!(
        "           Since: {}",
        if user.since > 0 {
            format_timestamp(user.since)
        } else {
            "-".to_string()
        }
    );
    println!("           State: {}", user.state);
    println!(
        "        Sessions: {}",
        if user.sessions.is_empty() {
            "(none)".to_string()
        } else {
            user.sessions.join(" ")
        }
    );
    println!("           Slice: {}", user.slice);
}

fn format_timestamp(epoch_secs: u64) -> String {
    // Simple timestamp formatting
    let secs = epoch_secs;
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Rough date calculation (not accounting for leap years precisely)
    let mut year = 1970u64;
    let mut remaining_days = days_since_epoch;
    loop {
        let days_in_year =
            if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
                366
            } else {
                365
            };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let days_in_months: &[u64] =
        if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
            &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };

    let mut month = 1u64;
    for &dim in days_in_months {
        if remaining_days < dim {
            break;
        }
        remaining_days -= dim;
        month += 1;
    }
    let day = remaining_days + 1;

    format!(
        "{}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, month, day, hours, minutes, seconds
    )
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Filter out known flags
    let mut no_legend = false;
    let mut no_pager = false;
    let mut full = false;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                usage();
                process::exit(0);
            }
            "--no-legend" => no_legend = true,
            "--no-pager" => no_pager = true,
            "-l" | "--full" => full = true,
            "-a" | "--all" => { /* accept but ignore for now */ }
            "--no-ask-password" => { /* accept but ignore */ }
            "-p" | "--property" => {
                i += 1; // skip the property value
            }
            "-H" | "--host" => {
                i += 1; // skip the host value
            }
            "-M" | "--machine" => {
                i += 1; // skip the machine value
            }
            "--system" | "--user" => { /* accept */ }
            arg if arg.starts_with('-') => {
                eprintln!("Unknown option: {}", arg);
                process::exit(1);
            }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }

    // Suppress unused variable warnings
    let _ = no_pager;
    let _ = full;

    if positional.is_empty() {
        positional.push("list-sessions".to_string());
    }

    let command = positional[0].as_str();
    let arg = positional.get(1).map(|s| s.as_str()).unwrap_or("");

    let exit_code = match command {
        "list-sessions" => match send_command("list-sessions") {
            Ok(resp) => {
                if resp.starts_with("ERROR") {
                    eprintln!("{}", resp);
                    1
                } else {
                    print_sessions(&resp, no_legend);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to list sessions: {}", e);
                1
            }
        },

        "list-seats" => match send_command("list-seats") {
            Ok(resp) => {
                if resp.starts_with("ERROR") {
                    eprintln!("{}", resp);
                    1
                } else {
                    print_seats(&resp, no_legend);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to list seats: {}", e);
                1
            }
        },

        "list-users" => match send_command("list-users") {
            Ok(resp) => {
                if resp.starts_with("ERROR") {
                    eprintln!("{}", resp);
                    1
                } else {
                    print_users(&resp, no_legend);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to list users: {}", e);
                1
            }
        },

        "list-inhibitors" => match send_command("list-inhibitors") {
            Ok(resp) => {
                if resp.starts_with("ERROR") {
                    eprintln!("{}", resp);
                    1
                } else {
                    print_inhibitors(&resp, no_legend);
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to list inhibitors: {}", e);
                1
            }
        },

        "session-status" | "show-session" => {
            if arg.is_empty() {
                eprintln!("Session ID required");
                1
            } else {
                let cmd = format!("show-session {}", arg);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            print_session_status(&resp);
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to show session: {}", e);
                        1
                    }
                }
            }
        }

        "seat-status" | "show-seat" => {
            let seat_name = if arg.is_empty() { "seat0" } else { arg };
            let cmd = format!("show-seat {}", seat_name);
            match send_command(&cmd) {
                Ok(resp) => {
                    if resp.starts_with("ERROR") {
                        eprintln!("{}", resp);
                        1
                    } else {
                        print_seat_status(&resp);
                        0
                    }
                }
                Err(e) => {
                    eprintln!("Failed to show seat: {}", e);
                    1
                }
            }
        }

        "user-status" | "show-user" => {
            if arg.is_empty() {
                // Default to current user
                let uid = unsafe { libc::getuid() };
                let cmd = format!("show-user {}", uid);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            print_user_status(&resp);
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to show user: {}", e);
                        1
                    }
                }
            } else {
                let cmd = format!("show-user {}", arg);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            print_user_status(&resp);
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to show user: {}", e);
                        1
                    }
                }
            }
        }

        "activate" => {
            if arg.is_empty() {
                eprintln!("Session ID required");
                1
            } else {
                let cmd = format!("activate-session {}", arg);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to activate session: {}", e);
                        1
                    }
                }
            }
        }

        "lock-session" => {
            if arg.is_empty() {
                eprintln!("Session ID required");
                1
            } else {
                let cmd = format!("lock-session {}", arg);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to lock session: {}", e);
                        1
                    }
                }
            }
        }

        "unlock-session" => {
            if arg.is_empty() {
                eprintln!("Session ID required");
                1
            } else {
                let cmd = format!("unlock-session {}", arg);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to unlock session: {}", e);
                        1
                    }
                }
            }
        }

        "terminate-session" | "kill-session" => {
            if arg.is_empty() {
                eprintln!("Session ID required");
                1
            } else {
                let cmd = format!("terminate-session {}", arg);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to terminate session: {}", e);
                        1
                    }
                }
            }
        }

        "terminate-user" | "kill-user" => {
            if arg.is_empty() {
                eprintln!("User (UID or name) required");
                1
            } else {
                // Resolve username to UID if needed
                let uid_str = if arg.parse::<u32>().is_ok() {
                    arg.to_string()
                } else {
                    // Try to look up user by name
                    resolve_username_to_uid(arg)
                        .map(|uid| uid.to_string())
                        .unwrap_or_else(|| arg.to_string())
                };
                let cmd = format!("terminate-user {}", uid_str);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to terminate user: {}", e);
                        1
                    }
                }
            }
        }

        "lock-sessions" => {
            // Lock all sessions — query list, then lock each
            match send_command("list-sessions") {
                Ok(resp) => {
                    if let Ok(sessions) = serde_json::from_str::<Vec<serde_json::Value>>(&resp) {
                        for s in &sessions {
                            if let Some(id) = s["id"].as_str() {
                                let _ = send_command(&format!("lock-session {}", id));
                            }
                        }
                        0
                    } else {
                        eprintln!("Failed to parse sessions");
                        1
                    }
                }
                Err(e) => {
                    eprintln!("Failed to list sessions: {}", e);
                    1
                }
            }
        }

        "unlock-sessions" => match send_command("list-sessions") {
            Ok(resp) => {
                if let Ok(sessions) = serde_json::from_str::<Vec<serde_json::Value>>(&resp) {
                    for s in &sessions {
                        if let Some(id) = s["id"].as_str() {
                            let _ = send_command(&format!("unlock-session {}", id));
                        }
                    }
                    0
                } else {
                    eprintln!("Failed to parse sessions");
                    1
                }
            }
            Err(e) => {
                eprintln!("Failed to list sessions: {}", e);
                1
            }
        },

        "poweroff"
        | "reboot"
        | "suspend"
        | "hibernate"
        | "hybrid-sleep"
        | "suspend-then-hibernate" => match send_command(command) {
            Ok(resp) => {
                if resp.starts_with("ERROR") {
                    eprintln!("{}", resp);
                    1
                } else {
                    0
                }
            }
            Err(e) => {
                eprintln!("Failed to {}: {}", command, e);
                1
            }
        },

        "enable-linger" => {
            if arg.is_empty() {
                eprintln!("User argument required");
                1
            } else {
                // Resolve username to UID if needed
                let uid_str = if arg.parse::<u32>().is_ok() {
                    arg.to_string()
                } else {
                    resolve_username_to_uid(arg)
                        .map(|uid| uid.to_string())
                        .unwrap_or_else(|| arg.to_string())
                };
                let cmd = format!("enable-linger {}", uid_str);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to enable linger: {}", e);
                        1
                    }
                }
            }
        }

        "disable-linger" => {
            if arg.is_empty() {
                eprintln!("User argument required");
                1
            } else {
                let uid_str = if arg.parse::<u32>().is_ok() {
                    arg.to_string()
                } else {
                    resolve_username_to_uid(arg)
                        .map(|uid| uid.to_string())
                        .unwrap_or_else(|| arg.to_string())
                };
                let cmd = format!("disable-linger {}", uid_str);
                match send_command(&cmd) {
                    Ok(resp) => {
                        if resp.starts_with("ERROR") {
                            eprintln!("{}", resp);
                            1
                        } else {
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to disable linger: {}", e);
                        1
                    }
                }
            }
        }

        "help" => {
            usage();
            0
        }

        _ => {
            eprintln!("Unknown command: {}", command);
            eprintln!("Use 'loginctl --help' for available commands.");
            1
        }
    };

    process::exit(exit_code);
}

/// Resolve a username to a UID using getpwnam.
fn resolve_username_to_uid(name: &str) -> Option<u32> {
    use std::ffi::CString;
    let c_name = CString::new(name).ok()?;
    unsafe {
        let pwd = libc::getpwnam(c_name.as_ptr());
        if pwd.is_null() {
            None
        } else {
            Some((*pwd).pw_uid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp() {
        let ts = format_timestamp(0);
        assert!(ts.contains("1970-01-01"));
        assert!(ts.contains("00:00:00"));
    }

    #[test]
    fn test_format_timestamp_nonzero() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        let ts = format_timestamp(1704067200);
        assert!(ts.contains("2024"));
        assert!(ts.contains("01-01"));
    }

    #[test]
    fn test_resolve_nonexistent_user() {
        let result = resolve_username_to_uid("this_user_definitely_does_not_exist_12345");
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_root_user() {
        let result = resolve_username_to_uid("root");
        assert_eq!(result, Some(0));
    }

    #[test]
    fn test_print_sessions_empty() {
        // Should not panic
        print_sessions("[]", true);
    }

    #[test]
    fn test_print_seats_empty() {
        print_seats("[]", true);
    }

    #[test]
    fn test_print_users_empty() {
        print_users("[]", true);
    }

    #[test]
    fn test_print_inhibitors_empty() {
        print_inhibitors("[]", true);
    }

    #[test]
    fn test_print_sessions_invalid_json() {
        // Should print error, not panic
        print_sessions("not json", false);
    }

    #[test]
    fn test_print_session_status_valid() {
        let json = r#"{
            "id": "1",
            "uid": 1000,
            "user": "testuser",
            "seat": "seat0",
            "tty": "/dev/tty1",
            "state": "active",
            "active": true,
            "vtnr": 1,
            "session_type": "tty",
            "class": "user",
            "scope": "pam",
            "leader": 1234,
            "since": 1704067200
        }"#;
        // Should not panic
        print_session_status(json);
    }

    #[test]
    fn test_print_seat_status_valid() {
        let json = r#"{
            "id": "seat0",
            "active_session": "1",
            "sessions": ["1", "2"],
            "can_graphical": true,
            "can_multi_session": true
        }"#;
        print_seat_status(json);
    }

    #[test]
    fn test_print_user_status_valid() {
        let json = r#"{
            "uid": 1000,
            "name": "testuser",
            "state": "active",
            "sessions": ["1"],
            "slice": "user-1000.slice",
            "since": 1704067200
        }"#;
        print_user_status(json);
    }
}
