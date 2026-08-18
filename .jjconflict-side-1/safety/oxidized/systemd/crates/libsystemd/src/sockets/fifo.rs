use std::{
    os::unix::io::AsRawFd, os::unix::io::FromRawFd, os::unix::io::IntoRawFd, os::unix::io::RawFd,
};

use log::trace;

use crate::units::SocketConfig;

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct FifoConfig {
    pub path: std::path::PathBuf,
}

impl FifoConfig {
    pub fn open(&self, conf: &SocketConfig) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)
                .map_err(|e| format!("Error removing file {:?}: {}", self.path, e))?;
        }
        let mode = nix::sys::stat::Mode::S_IRWXU;
        nix::unistd::mkfifo(&self.path, mode)
            .map_err(|e| format!("Error while creating fifo {:?}: {}", &self.path, e))?;

        // open NON_BLOCK so we dont wait for the other end of the fifo
        let mut open_flags = nix::fcntl::OFlag::empty();
        open_flags.insert(nix::fcntl::OFlag::O_RDWR);
        //open_flags.insert(nix::fcntl::OFlag::O_NONBLOCK);
        let fifo_fd = nix::fcntl::open(&self.path, open_flags, mode)
            .map_err(|e| format!("Error opening fifo file {:?}: {}", self.path, e))?;
        // need to make a file out of that so AsRawFd is implemented (it's not implemented for RawFd itself...)
        let raw_fd = fifo_fd.into_raw_fd();

        // Apply PipeSize= (F_SETPIPE_SZ)
        if let Some(pipe_size) = conf.pipe_size {
            let ret = unsafe { libc::fcntl(raw_fd, libc::F_SETPIPE_SZ, pipe_size as libc::c_int) };
            if ret < 0 {
                trace!(
                    "Failed to set F_SETPIPE_SZ={} on FIFO {:?}: {}",
                    pipe_size,
                    self.path,
                    std::io::Error::last_os_error()
                );
            }
        }

        // Apply SocketUser=/SocketGroup=/SocketMode=
        super::apply_socket_ownership(&self.path, conf);

        let fifo = unsafe { std::fs::File::from_raw_fd(raw_fd) };
        Ok(Box::new(fifo))
    }

    pub fn close(&self, _rawfd: RawFd, remove_on_stop: bool) -> Result<(), String> {
        if remove_on_stop && self.path.exists() {
            std::fs::remove_file(&self.path)
                .map_err(|e| format!("Error removing file {:?}: {}", self.path, e))?;
        }
        // fd is closed by OwnedFd::drop when the Box<dyn AsRawFd> is dropped
        Ok(())
    }
}
