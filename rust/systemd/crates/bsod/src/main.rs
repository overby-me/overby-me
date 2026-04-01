//! systemd-bsod — Display emergency log messages on the virtual console.
//!
//! A drop-in replacement for `systemd-bsod(8)`. Reads the journal for the
//! first emergency-level message from the current boot (UID=0) and displays
//! it on a free virtual terminal with a blue background.

use clap::Parser;
use libsystemd::journal::storage::{JournalStorage, StorageConfig};
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

/// Find the first emergency message from the current boot with UID=0.
fn find_emergency_message(continuous: bool) -> Option<String> {
    let boot_id = read_boot_id();

    let directory = if PathBuf::from("/var/log/journal").is_dir() {
        "/var/log/journal".into()
    } else {
        "/run/log/journal".into()
    };

    let config = StorageConfig {
        directory,
        max_file_size: u64::MAX,
        max_disk_usage: u64::MAX,
        max_files: usize::MAX,
        persistent: false,
        keep_free: 0,
        direct_directory: false,
        ..Default::default()
    };

    let storage = match JournalStorage::open_read_only(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open journal: {e}");
            return None;
        }
    };

    let entries = match storage.read_all() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to read journal: {e}");
            return None;
        }
    };

    // Filter: current boot, UID=0, PRIORITY=0 (emerg)
    for entry in &entries {
        if entry.boot_id().as_deref() != Some(&boot_id) {
            continue;
        }
        if entry.uid() != Some(0) {
            continue;
        }
        if entry.priority() != Some(0) {
            continue;
        }
        if let Some(msg) = entry.field("MESSAGE") {
            return Some(msg);
        }
    }

    if continuous {
        // In continuous mode, wait for new entries by polling.
        // For simplicity, poll with a sleep loop.
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));

            // Re-read journal
            let config2 = StorageConfig {
                directory: if PathBuf::from("/var/log/journal").is_dir() {
                    "/var/log/journal".into()
                } else {
                    "/run/log/journal".into()
                },
                max_file_size: u64::MAX,
                max_disk_usage: u64::MAX,
                max_files: usize::MAX,
                persistent: false,
                keep_free: 0,
                direct_directory: false,
                ..Default::default()
            };
            let storage2 = match JournalStorage::open_read_only(config2) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let entries2 = match storage2.read_all() {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in &entries2 {
                if entry.boot_id().as_deref() != Some(&boot_id) {
                    continue;
                }
                if entry.uid() != Some(0) {
                    continue;
                }
                if entry.priority() != Some(0) {
                    continue;
                }
                if let Some(msg) = entry.field("MESSAGE") {
                    return Some(msg);
                }
            }

            // Check if we received a signal
            if check_signal() {
                return None;
            }
        }
    }

    None
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

/// Find the next free virtual terminal.
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
    for i in 0..16u16 {
        if (state.v_state & (1 << i)) == 0 {
            return Ok((i as i32 + 1, state.v_active as i32));
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "No free VT"))
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
    let (fd, free_vt, original_vt) = if let Some(tty) = tty_path {
        let f = fs::OpenOptions::new().read(true).write(true).open(tty)?;
        let raw_fd = f.as_raw_fd();
        // Leak the file to keep fd open
        std::mem::forget(f);
        (raw_fd, 0, 0)
    } else {
        // Open /dev/tty1 to find a free VT
        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty1")?;
        let raw_fd = f.as_raw_fd();
        let (free_vt, original_vt) = find_free_vt(raw_fd)?;
        drop(f);

        let tty_name = format!("/dev/tty{free_vt}");
        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&tty_name)?;
        let raw_fd = f.as_raw_fd();
        std::mem::forget(f);

        // Activate the free VT: VT_ACTIVATE = 0x5606
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
        let _ = set_cursor_position(fd, qr_row, qr_col);
        let qr_header = "Scan the error message";
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
