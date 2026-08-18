//! systemd-random-seed — Load/save the OS random seed across reboots.
//!
//! A drop-in replacement for `systemd-random-seed(8)`. This tool is invoked
//! by the `systemd-random-seed.service` unit:
//!
//!   ExecStart=systemd-random-seed load   → credit saved seed to kernel RNG
//!   ExecStop=systemd-random-seed save    → save kernel RNG state to disk
//!
//! The seed file is stored at `/var/lib/systemd/random-seed` (512 bytes).
//!
//! On `load`:
//!   1. Read the saved seed file (if it exists).
//!   2. Write the seed data to `/dev/urandom` to credit the kernel entropy pool.
//!   3. Optionally use the `RNDADDENTROPY` ioctl to credit entropy bits.
//!   4. Immediately refresh the seed file with new random data from `/dev/urandom`
//!      so the same seed is never used twice.
//!
//! On `save`:
//!   1. Read 512 bytes from `/dev/urandom`.
//!   2. Write them to the seed file.
//!
//! The seed file is created with mode 0o600 and the directory with 0o755.
//!
//! Exit codes:
//!   0 — success
//!   1 — error

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process;

const SEED_DIR: &str = "/var/lib/systemd";
const SEED_FILE: &str = "/var/lib/systemd/random-seed";
const URANDOM_PATH: &str = "/dev/urandom";
const SEED_SIZE: usize = 512;

/// The RNDADDENTROPY ioctl number (Linux).
/// This credits entropy bits to the kernel random pool.
/// struct rand_pool_info { int entropy_count; int buf_size; __u32 buf[0]; }
#[cfg(target_os = "linux")]
const RNDADDENTROPY: libc::c_ulong = 0x40085203;

/// Credit entropy to the kernel pool using the RNDADDENTROPY ioctl.
///
/// This tells the kernel that the data we wrote contains real entropy,
/// which helps the RNG become initialized faster during early boot.
#[cfg(target_os = "linux")]
fn credit_entropy(data: &[u8]) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;

    let f = fs::File::open(URANDOM_PATH)?;
    let fd = f.as_raw_fd();

    // Build the rand_pool_info structure:
    //   int entropy_count  (in bits)
    //   int buf_size        (in bytes)
    //   __u32 buf[]
    let entropy_bits = (data.len() * 8) as i32;
    let buf_size = data.len() as i32;

    // Allocate buffer: 2 ints (8 bytes) + data
    let mut info = Vec::with_capacity(8 + data.len());
    info.extend_from_slice(&entropy_bits.to_ne_bytes());
    info.extend_from_slice(&buf_size.to_ne_bytes());
    info.extend_from_slice(data);

    let ret = unsafe { libc::ioctl(fd, RNDADDENTROPY, info.as_ptr()) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn credit_entropy(_data: &[u8]) -> io::Result<()> {
    Ok(())
}

/// Ensure the seed directory exists with the correct permissions.
fn ensure_seed_dir(dir: &Path) -> io::Result<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// Read the saved seed file. Returns the seed data or None if the file
/// doesn't exist or can't be read.
fn read_seed(seed_path: &Path) -> Option<Vec<u8>> {
    match fs::read(seed_path) {
        Ok(data) if !data.is_empty() => Some(data),
        Ok(_) => {
            eprintln!("Seed file {} is empty, ignoring.", seed_path.display());
            None
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            eprintln!(
                "No seed file {} found, starting without saved entropy.",
                seed_path.display()
            );
            None
        }
        Err(e) => {
            eprintln!(
                "Warning: failed to read seed file {}: {}",
                seed_path.display(),
                e
            );
            None
        }
    }
}

/// Write seed data to /dev/urandom to mix it into the kernel entropy pool.
fn write_to_urandom(data: &[u8], urandom_path: &Path) -> io::Result<()> {
    let mut f = fs::OpenOptions::new().write(true).open(urandom_path)?;
    f.write_all(data)?;
    Ok(())
}

/// Read random bytes from /dev/urandom.
fn read_from_urandom(size: usize, urandom_path: &Path) -> io::Result<Vec<u8>> {
    let mut f = fs::File::open(urandom_path)?;
    let mut buf = vec![0u8; size];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Write a new seed file with the given data.
fn write_seed(seed_path: &Path, data: &[u8]) -> io::Result<()> {
    // Write atomically: write to a temp file, then rename.
    let dir = seed_path
        .parent()
        .ok_or_else(|| io::Error::other("seed path has no parent"))?;
    let tmp_path = dir.join(".random-seed.tmp");

    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        f.write_all(data)?;
        f.sync_all()?;
    }

    fs::rename(&tmp_path, seed_path)?;
    Ok(())
}

/// Load the random seed: credit it to the kernel RNG and refresh the seed file.
fn load(seed_path: &Path, urandom_path: &Path, seed_dir: &Path) -> i32 {
    // Read the existing seed.
    if let Some(seed_data) = read_seed(seed_path) {
        // Write to /dev/urandom.
        match write_to_urandom(&seed_data, urandom_path) {
            Ok(()) => {
                eprintln!(
                    "Loaded {} bytes from {} into kernel entropy pool.",
                    seed_data.len(),
                    seed_path.display()
                );
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to write seed to {}: {}",
                    urandom_path.display(),
                    e
                );
            }
        }

        // Try to credit entropy bits via ioctl (requires CAP_SYS_ADMIN).
        match credit_entropy(&seed_data) {
            Ok(()) => {
                eprintln!(
                    "Credited {} bits of entropy to the kernel.",
                    seed_data.len() * 8
                );
            }
            Err(e) => {
                // This is expected to fail without CAP_SYS_ADMIN; not fatal.
                eprintln!(
                    "Note: could not credit entropy via ioctl: {} (not fatal)",
                    e
                );
            }
        }
    }

    // Immediately refresh the seed file with new random data so the
    // same seed is never reused across boots.
    if let Err(e) = ensure_seed_dir(seed_dir) {
        eprintln!(
            "Warning: failed to create seed directory {}: {}",
            seed_dir.display(),
            e
        );
        return 0; // Non-fatal
    }

    match read_from_urandom(SEED_SIZE, urandom_path) {
        Ok(new_seed) => match write_seed(seed_path, &new_seed) {
            Ok(()) => {
                eprintln!(
                    "Refreshed seed file {} with {} fresh bytes.",
                    seed_path.display(),
                    SEED_SIZE
                );
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to refresh seed file {}: {}",
                    seed_path.display(),
                    e
                );
            }
        },
        Err(e) => {
            eprintln!(
                "Warning: failed to read from {}: {}",
                urandom_path.display(),
                e
            );
        }
    }

    0
}

/// Save the current kernel random state to the seed file.
fn save(seed_path: &Path, urandom_path: &Path, seed_dir: &Path) -> i32 {
    if let Err(e) = ensure_seed_dir(seed_dir) {
        eprintln!(
            "Error: failed to create seed directory {}: {}",
            seed_dir.display(),
            e
        );
        return 1;
    }

    match read_from_urandom(SEED_SIZE, urandom_path) {
        Ok(data) => match write_seed(seed_path, &data) {
            Ok(()) => {
                eprintln!(
                    "Saved {} bytes of random seed to {}.",
                    SEED_SIZE,
                    seed_path.display()
                );
                0
            }
            Err(e) => {
                eprintln!(
                    "Error: failed to write seed file {}: {}",
                    seed_path.display(),
                    e
                );
                1
            }
        },
        Err(e) => {
            eprintln!(
                "Error: failed to read from {}: {}",
                urandom_path.display(),
                e
            );
            1
        }
    }
}

fn usage() -> ! {
    eprintln!("Usage: systemd-random-seed {{load|save}}");
    eprintln!();
    eprintln!("  load   Credit saved random seed to kernel RNG and refresh seed file");
    eprintln!("  save   Save kernel RNG state to seed file for next boot");
    process::exit(1);
}

fn run(args: &[String], seed_path: &Path, urandom_path: &Path, seed_dir: &Path) -> i32 {
    let command = match args.get(1) {
        Some(cmd) => cmd.as_str(),
        None => usage(),
    };

    match command {
        "load" => load(seed_path, urandom_path, seed_dir),
        "save" => save(seed_path, urandom_path, seed_dir),
        other => {
            eprintln!("Unknown command: {}", other);
            usage();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed_path = Path::new(SEED_FILE);
    let urandom_path = Path::new(URANDOM_PATH);
    let seed_dir = Path::new(SEED_DIR);

    let code = run(&args, seed_path, urandom_path, seed_dir);
    process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("failed to create temp dir")
    }

    #[test]
    fn test_constants() {
        assert_eq!(SEED_DIR, "/var/lib/systemd");
        assert_eq!(SEED_FILE, "/var/lib/systemd/random-seed");
        assert_eq!(URANDOM_PATH, "/dev/urandom");
        assert_eq!(SEED_SIZE, 512);
    }

    #[test]
    fn test_ensure_seed_dir_creates_missing() {
        let dir = temp_dir();
        let seed_dir = dir.path().join("a/b/c");
        assert!(!seed_dir.exists());

        ensure_seed_dir(&seed_dir).unwrap();
        assert!(seed_dir.is_dir());
    }

    #[test]
    fn test_ensure_seed_dir_existing_ok() {
        let dir = temp_dir();
        ensure_seed_dir(dir.path()).unwrap();
        assert!(dir.path().is_dir());
    }

    #[test]
    fn test_read_seed_existing() {
        let dir = temp_dir();
        let seed = dir.path().join("random-seed");
        let data = vec![42u8; SEED_SIZE];
        fs::write(&seed, &data).unwrap();

        let result = read_seed(&seed);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), data);
    }

    #[test]
    fn test_read_seed_missing() {
        let dir = temp_dir();
        let seed = dir.path().join("nonexistent");
        assert!(read_seed(&seed).is_none());
    }

    #[test]
    fn test_read_seed_empty() {
        let dir = temp_dir();
        let seed = dir.path().join("empty-seed");
        fs::write(&seed, b"").unwrap();
        assert!(read_seed(&seed).is_none());
    }

    #[test]
    fn test_write_seed_creates_file() {
        let dir = temp_dir();
        let seed = dir.path().join("random-seed");
        let data = vec![0xAB; 64];

        write_seed(&seed, &data).unwrap();
        assert!(seed.exists());

        let read_back = fs::read(&seed).unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn test_write_seed_overwrites() {
        let dir = temp_dir();
        let seed = dir.path().join("random-seed");
        fs::write(&seed, b"old data").unwrap();

        let new_data = vec![0xCD; 128];
        write_seed(&seed, &new_data).unwrap();

        let read_back = fs::read(&seed).unwrap();
        assert_eq!(read_back, new_data);
    }

    #[test]
    fn test_write_seed_permissions() {
        let dir = temp_dir();
        let seed = dir.path().join("random-seed");
        write_seed(&seed, &[1, 2, 3]).unwrap();

        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(&seed).unwrap();
        let mode = meta.mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn test_read_from_urandom() {
        // /dev/urandom should be available on any Linux/macOS test host.
        let urandom = Path::new("/dev/urandom");
        if !urandom.exists() {
            return; // Skip on systems without /dev/urandom
        }

        let data = read_from_urandom(64, urandom).unwrap();
        assert_eq!(data.len(), 64);

        // Very unlikely that 64 random bytes are all zeros.
        assert!(data.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_write_to_urandom() {
        let urandom = Path::new("/dev/urandom");
        if !urandom.exists() {
            return;
        }

        // Writing to /dev/urandom should succeed (it mixes data into the pool).
        let data = vec![42u8; 32];
        write_to_urandom(&data, urandom).unwrap();
    }

    #[test]
    fn test_save_creates_seed() {
        let dir = temp_dir();
        let seed = dir.path().join("random-seed");
        let urandom = Path::new("/dev/urandom");
        if !urandom.exists() {
            return;
        }

        let code = save(&seed, urandom, dir.path());
        assert_eq!(code, 0);
        assert!(seed.exists());

        let data = fs::read(&seed).unwrap();
        assert_eq!(data.len(), SEED_SIZE);
    }

    #[test]
    fn test_load_no_existing_seed() {
        let dir = temp_dir();
        let seed = dir.path().join("random-seed");
        let urandom = Path::new("/dev/urandom");
        if !urandom.exists() {
            return;
        }

        // Load with no existing seed should still succeed (just generate a new one).
        let code = load(&seed, urandom, dir.path());
        assert_eq!(code, 0);

        // A new seed file should have been created.
        assert!(seed.exists());
        let data = fs::read(&seed).unwrap();
        assert_eq!(data.len(), SEED_SIZE);
    }

    #[test]
    fn test_load_with_existing_seed() {
        let dir = temp_dir();
        let seed = dir.path().join("random-seed");
        let urandom = Path::new("/dev/urandom");
        if !urandom.exists() {
            return;
        }

        // Create an initial seed.
        let initial = vec![0xFFu8; SEED_SIZE];
        fs::write(&seed, &initial).unwrap();

        let code = load(&seed, urandom, dir.path());
        assert_eq!(code, 0);

        // Seed should have been refreshed (different from initial).
        let refreshed = fs::read(&seed).unwrap();
        assert_eq!(refreshed.len(), SEED_SIZE);
        // It's astronomically unlikely that 512 random bytes equal 512 0xFF bytes.
        assert_ne!(refreshed, initial);
    }

    #[test]
    fn test_load_then_save_roundtrip() {
        let dir = temp_dir();
        let seed = dir.path().join("random-seed");
        let urandom = Path::new("/dev/urandom");
        if !urandom.exists() {
            return;
        }

        let code = load(&seed, urandom, dir.path());
        assert_eq!(code, 0);

        let after_load = fs::read(&seed).unwrap();

        let code = save(&seed, urandom, dir.path());
        assert_eq!(code, 0);

        let after_save = fs::read(&seed).unwrap();
        assert_eq!(after_save.len(), SEED_SIZE);
        // Save should have written new random data.
        assert_ne!(after_save, after_load);
    }

    #[test]
    fn test_save_creates_missing_directory() {
        let dir = temp_dir();
        let seed_dir = dir.path().join("lib/systemd");
        let seed = seed_dir.join("random-seed");
        let urandom = Path::new("/dev/urandom");
        if !urandom.exists() {
            return;
        }

        let code = save(&seed, urandom, &seed_dir);
        assert_eq!(code, 0);
        assert!(seed_dir.is_dir());
        assert!(seed.exists());
    }

    #[test]
    fn test_seed_file_not_reused() {
        let dir = temp_dir();
        let seed = dir.path().join("random-seed");
        let urandom = Path::new("/dev/urandom");
        if !urandom.exists() {
            return;
        }

        // Simulate two boots: save, then load.
        save(&seed, urandom, dir.path());
        let seed_after_save = fs::read(&seed).unwrap();

        load(&seed, urandom, dir.path());
        let seed_after_load = fs::read(&seed).unwrap();

        // The seed must be different after load (to prevent reuse).
        assert_ne!(seed_after_save, seed_after_load);
    }
}
