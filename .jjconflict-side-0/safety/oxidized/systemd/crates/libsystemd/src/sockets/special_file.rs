use std::{
    os::unix::io::AsRawFd, os::unix::io::FromRawFd, os::unix::io::IntoRawFd, os::unix::io::RawFd,
};

use crate::units::SocketConfig;

/// Configuration for a `ListenSpecial=` socket entry.
///
/// `ListenSpecial=` in systemd listens on special files in `/proc`, `/sys`,
/// or on device nodes. The file is opened with `O_RDONLY|O_CLOEXEC|O_NOCTTY`
/// (or `O_RDWR` if `Writable=yes` is set on the socket unit).
///
/// Unlike FIFOs, the file is NOT created — it must already exist. The file
/// is simply opened and the resulting file descriptor is passed to the
/// activated service.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct SpecialFileConfig {
    pub path: std::path::PathBuf,
}

impl SpecialFileConfig {
    pub fn open(&self, _conf: &SocketConfig) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
        // Open the special file read-only with O_CLOEXEC and O_NOCTTY.
        // Note: Writable= support (O_RDWR) is handled at a higher level;
        // for now we open O_RDONLY which is the systemd default.
        let mut open_flags = nix::fcntl::OFlag::empty();
        open_flags.insert(nix::fcntl::OFlag::O_RDONLY);
        open_flags.insert(nix::fcntl::OFlag::O_CLOEXEC);
        open_flags.insert(nix::fcntl::OFlag::O_NOCTTY);

        let mode = nix::sys::stat::Mode::empty();
        let fd = nix::fcntl::open(&self.path, open_flags, mode)
            .map_err(|e| format!("Error opening special file {:?}: {}", self.path, e))?;
        let file = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
        Ok(Box::new(file))
    }

    pub fn close(&self, _rawfd: RawFd) -> Result<(), String> {
        // fd is closed by OwnedFd::drop when the Box<dyn AsRawFd> is dropped.
        // For special files we do NOT remove the file (unlike FIFOs).
        Ok(())
    }
}
