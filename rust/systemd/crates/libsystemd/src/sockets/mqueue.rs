//! POSIX message queue socket support for `ListenMessageQueue=`.
//!
//! A `ListenMessageQueue=/name` directive on a socket unit causes the
//! service manager to create a POSIX message queue at `/name` (visible
//! as `/dev/mqueue/name` when the `mqueue` filesystem is mounted) and
//! pass its file descriptor to the activated service as an ordinary
//! socket-activation FD.

use log::trace;
use std::ffi::CString;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

use crate::units::SocketConfig;

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct MessageQueueConfig {
    /// The queue name, including the leading '/'.
    pub name: String,
    /// Optional message queue attributes from MessageQueueMsgs= / MessageQueueMsgSize=.
    pub max_messages: Option<i64>,
    pub message_size: Option<i64>,
}

impl MessageQueueConfig {
    pub fn open(&self, conf: &SocketConfig) -> Result<Box<dyn AsRawFd + Send + Sync>, String> {
        // mq_open wants a C string starting with '/'.  If the queue
        // already exists we unlink it first so we own the creation and
        // can set explicit ownership.
        let cname = CString::new(self.name.as_str())
            .map_err(|e| format!("invalid queue name {}: {e}", self.name))?;
        unsafe { libc::mq_unlink(cname.as_ptr()) };

        // Pick file mode from SocketMode=, falling back to 0600.
        let mode: libc::mode_t = conf.socket_mode.unwrap_or(0o0600) as libc::mode_t;

        // Build mq_attr if the user gave us explicit limits.
        // `libc::mq_attr` has private fields so we zero-init via mem::zeroed
        // and then set the public fields.
        let mut attr: libc::mq_attr = unsafe { std::mem::zeroed() };
        attr.mq_maxmsg = self.max_messages.unwrap_or(10);
        attr.mq_msgsize = self.message_size.unwrap_or(8192);
        let attr_ptr: *mut libc::mq_attr =
            if self.max_messages.is_some() || self.message_size.is_some() {
                &mut attr
            } else {
                std::ptr::null_mut()
            };

        // O_RDWR so both sender and receiver FDs are the same queue fd.
        let flags = libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC;
        let fd = unsafe { libc::mq_open(cname.as_ptr(), flags, mode as libc::c_uint, attr_ptr) }
            as RawFd;
        if fd < 0 {
            return Err(format!(
                "mq_open({}) failed: {}",
                self.name,
                std::io::Error::last_os_error()
            ));
        }

        // Apply SocketUser=/SocketGroup=/SocketMode= to the underlying
        // /dev/mqueue entry so `stat -c` reports them.  This matches real
        // systemd's behaviour.  We only do this if /dev/mqueue exists —
        // not all setups have the mqueue filesystem mounted.
        let mq_path = std::path::PathBuf::from(format!("/dev/mqueue{}", self.name));
        if mq_path.exists() {
            super::apply_socket_ownership(&mq_path, conf);
        } else {
            trace!(
                "MessageQueue {}: /dev/mqueue not mounted, skipping ownership apply",
                self.name
            );
        }

        // Wrap the fd in an owned File so the dyn AsRawFd trait object
        // holds ownership (dropping it closes the fd).
        let owned = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
        Ok(Box::new(owned))
    }

    pub fn close(&self, rawfd: RawFd, remove_on_stop: bool) -> Result<(), String> {
        // Closing the fd alone does not unlink the queue — the name
        // persists in the global namespace until mq_unlink() is called.
        if remove_on_stop {
            let cname = CString::new(self.name.as_str())
                .map_err(|e| format!("invalid queue name {}: {e}", self.name))?;
            unsafe { libc::mq_unlink(cname.as_ptr()) };
        }
        unsafe { libc::close(rawfd) };
        Ok(())
    }
}
