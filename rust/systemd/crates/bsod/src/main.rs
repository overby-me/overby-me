//! systemd-bsod — Display emergency log messages on the virtual console.
//!
//! A drop-in replacement for `systemd-bsod(8)`. Reads the journal for the
//! first emergency-level message from the current boot (UID=0) and displays
//! it on a free virtual terminal with a blue background.

use clap::Parser;
use libsystemd::journal::storage::{list_all_journal_files, read_entries_from_offset};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

/// ANSI escape sequences for terminal control.
const ANSI_HOME_CLEAR: &str = "\x1b[H\x1b[2J";
const ANSI_BACKGROUND_BLUE: &str = "\x1b[44m\x1b[37m";

#[derive(Parser, Debug)]
#[command(
    name = "systemd-bsod",
    about = "Display emergency log messages on the console",
    version
)]
struct Cli {
    /// Wait continuously for emergency messages.
    #[arg(short = 'c', long = "continuous")]
    continuous: bool,

    /// Specify the TTY to use.
    #[arg(long = "tty")]
    tty: Option<PathBuf>,
}

/// Read the current boot ID from /proc.
fn read_boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|s| s.trim().replace('-', ""))
        .unwrap_or_else(|_| "0".repeat(32))
}

/// Resolve the journal directory (with machine-id subdirectory).
fn journal_directory() -> PathBuf {
    let base = if PathBuf::from("/var/log/journal").is_dir() {
        PathBuf::from("/var/log/journal")
    } else {
        PathBuf::from("/run/log/journal")
    };

    // Append machine-id subdirectory (matching JournalStorage layout)
    let machine_id = fs::read_to_string("/etc/machine-id")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();

    if machine_id.is_empty() {
        base
    } else {
        base.join(machine_id)
    }
}

/// Check if an entry matches our emergency message criteria.
fn is_emergency_match(entry: &libsystemd::journal::entry::JournalEntry, boot_id: &str) -> bool {
    entry.boot_id().as_deref() == Some(boot_id)
        && entry.uid() == Some(0)
        && entry.priority() == Some(0)
}

/// Find the first emergency message by scanning journal files incrementally.
/// Returns as soon as a match is found, without loading the entire journal.
fn find_emergency_message(continuous: bool) -> Option<String> {
    let boot_id = read_boot_id();
    let journal_dir = journal_directory();

    // First pass: scan all existing entries
    let files = list_all_journal_files(&journal_dir).unwrap_or_default();
    let mut offsets: HashMap<PathBuf, u64> = HashMap::new();

    for file in &files {
        match read_entries_from_offset(file, 0) {
            Ok((entries, end_offset)) => {
                offsets.insert(file.clone(), end_offset);
                for entry in &entries {
                    if is_emergency_match(entry, &boot_id)
                        && let Some(msg) = entry.field("MESSAGE")
                    {
                        return Some(msg);
                    }
                }
            }
            Err(_) => {
                // Try C journal format via read_all fallback for this file
                if let Ok(entries) = libsystemd::journal::c_journal::read_c_journal(file) {
                    for entry in &entries {
                        if is_emergency_match(entry, &boot_id)
                            && let Some(msg) = entry.field("MESSAGE")
                        {
                            return Some(msg);
                        }
                    }
                }
            }
        }
    }

    if !continuous {
        return None;
    }

    // Continuous mode: poll for new entries using incremental reads
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));

        if check_signal() {
            return None;
        }

        // Check for new files and new data in existing files
        let current_files = list_all_journal_files(&journal_dir).unwrap_or_default();

        for file in &current_files {
            let offset = offsets.get(file).copied().unwrap_or(0);
            if let Ok((entries, end_offset)) = read_entries_from_offset(file, offset) {
                offsets.insert(file.clone(), end_offset);
                for entry in &entries {
                    if is_emergency_match(entry, &boot_id)
                        && let Some(msg) = entry.field("MESSAGE")
                    {
                        return Some(msg);
                    }
                }
            }
        }
    }
}

/// Check if SIGTERM or SIGINT has been received.
fn check_signal() -> bool {
    // We install signal handlers in main; this checks a global flag.
    SIGNAL_RECEIVED.load(std::sync::atomic::Ordering::Relaxed)
}

use std::sync::atomic::{AtomicBool, Ordering};

static SIGNAL_RECEIVED: AtomicBool = AtomicBool::new(false);

extern "C" fn signal_handler(_sig: libc::c_int) {
    SIGNAL_RECEIVED.store(true, Ordering::Relaxed);
}

/// VT_GETSTATE ioctl structure.
#[repr(C)]
struct VtState {
    v_active: u16,
    v_signal: u16,
    v_state: u16,
}

/// Find the next free virtual terminal by scanning the v_state bitmap.
/// Matches the C systemd `find_next_free_vt()` implementation.
fn find_free_vt(fd: i32) -> io::Result<(i32, i32)> {
    let mut state = VtState {
        v_active: 0,
        v_signal: 0,
        v_state: 0,
    };
    // VT_GETSTATE = 0x5603
    if unsafe { libc::ioctl(fd, 0x5603, &mut state) } < 0 {
        return Err(io::Error::last_os_error());
    }

    // Find the first free VT by scanning the v_state bitmap (bit i = VT i+1)
    for i in 0..16 {
        if state.v_state & (1 << i) == 0 {
            return Ok((i + 1, state.v_active as i32));
        }
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "no free VT found"))
}

/// Set the terminal cursor position (1-based row, col).
fn set_cursor_position(fd: i32, row: u32, col: u32) -> io::Result<()> {
    let seq = format!("\x1b[{row};{col}H");
    let written = unsafe { libc::write(fd, seq.as_ptr() as *const libc::c_void, seq.len()) };
    if written < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Display the emergency message on a virtual terminal.
fn display_message(message: &str, tty_path: Option<&PathBuf>) -> io::Result<()> {
    let (fd, _free_vt, original_vt) = if let Some(tty) = tty_path {
        let f = fs::OpenOptions::new().read(true).write(true).open(tty)?;
        let raw_fd = f.as_raw_fd();
        // Leak the file to keep fd open
        std::mem::forget(f);
        (raw_fd, 0, 0)
    } else {
        // Open /dev/tty1 to query VT state, then close it before opening
        // the target — matching the C systemd-bsod approach.
        let (free_vt, original_vt) = {
            let console = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty1")?;
            let result = find_free_vt(console.as_raw_fd())?;
            drop(console);
            result
        };

        // Open the target VT
        let tty_name = format!("/dev/tty{free_vt}");
        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&tty_name)?;
        let raw_fd = f.as_raw_fd();
        std::mem::forget(f);

        // Activate the free VT on its own fd (matching C systemd-bsod).
        // VT_ACTIVATE = 0x5606
        if free_vt > 0 {
            unsafe {
                libc::ioctl(raw_fd, 0x5606, free_vt);
            }
        }

        (raw_fd, free_vt, original_vt)
    };

    // Get terminal size
    let mut winsize: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut winsize) } < 0 {
        winsize.ws_col = 80;
        winsize.ws_row = 25;
    }

    // Clear screen with blue background
    let clear = format!("{ANSI_BACKGROUND_BLUE}{ANSI_HOME_CLEAR}");
    let _ = unsafe { libc::write(fd, clear.as_ptr() as *const libc::c_void, clear.len()) };

    // Display header at row 2, col 4
    let _ = set_cursor_position(fd, 2, 4);
    let header = "The current boot has failed!";
    let _ = unsafe { libc::write(fd, header.as_ptr() as *const libc::c_void, header.len()) };

    // Display message at row 4, col 4
    let _ = set_cursor_position(fd, 4, 4);
    let _ = unsafe { libc::write(fd, message.as_ptr() as *const libc::c_void, message.len()) };

    // Display QR code header (matches C systemd's print_qrcode_full header text)
    if std::env::var("SYSTEMD_COLORS").is_ok() {
        let qr_row = (winsize.ws_row as u32 * 3) / 5;
        let qr_col = (winsize.ws_col as u32 * 3) / 4;
        let qr_header = "Scan the error message";
        // Clamp column so the header text doesn't wrap past the terminal edge
        let max_col = (winsize.ws_col as u32).saturating_sub(qr_header.len() as u32);
        let qr_col = qr_col.min(max_col).max(1);
        let _ = set_cursor_position(fd, qr_row, qr_col);
        let _ = unsafe {
            libc::write(
                fd,
                qr_header.as_ptr() as *const libc::c_void,
                qr_header.len(),
            )
        };
    }

    // Display "Press any key to exit..." near bottom
    let bottom_row = winsize.ws_row.saturating_sub(1).max(6) as u32;
    let bottom_col = (winsize.ws_col as u32 * 2) / 5;
    let _ = set_cursor_position(fd, bottom_row, bottom_col);
    let prompt = format!("{ANSI_BACKGROUND_BLUE}Press any key to exit...");
    let _ = unsafe { libc::write(fd, prompt.as_ptr() as *const libc::c_void, prompt.len()) };

    // Wait for keypress or signal
    let mut buf = [0u8; 1];
    // Use read() which will be interrupted by our signal handler (no SA_RESTART)
    let _ = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, 1) };

    // Switch back to original VT if needed
    if original_vt > 0 {
        unsafe {
            libc::ioctl(fd, 0x5606, original_vt);
        }
    }

    // Close fd
    unsafe {
        libc::close(fd);
    }

    Ok(())
}

fn main() {
    let cli = Cli::parse();

    // Install signal handlers without SA_RESTART so read() gets interrupted
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = signal_handler as usize;
        sa.sa_flags = 0; // No SA_RESTART
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }

    let message = find_emergency_message(cli.continuous);

    match message {
        Some(msg) => {
            if let Err(e) = display_message(&msg, cli.tty.as_ref()) {
                eprintln!("Failed to display message: {e}");
                std::process::exit(1);
            }
        }
        None => {
            // No emergency messages — exit cleanly
        }
    }
}
