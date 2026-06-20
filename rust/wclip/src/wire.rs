//! Minimal Wayland wire-protocol client.
//!
//! This speaks the Wayland protocol directly over the compositor's unix socket
//! — there is no dependency on `libwayland`. Only the pieces needed for a
//! clipboard tool are implemented: connecting, allocating object ids, building
//! and sending requests, reading events, and passing file descriptors over the
//! socket via `SCM_RIGHTS` ancillary data.
//!
//! Wire format reference: messages are a sequence of 32-bit words in host byte
//! order. Every message starts with an 8-byte header — the sender object id
//! (`u32`) followed by a word packing `(size << 16) | opcode`, where `size` is
//! the total message length in bytes including the header. Arguments follow,
//! each padded to a 32-bit boundary. File descriptors travel out-of-band in the
//! socket's ancillary data rather than in the message body.

use std::collections::VecDeque;
use std::env;
use std::io::{self, ErrorKind};
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::ptr;

/// The `wl_display` object always has id 1.
pub const DISPLAY_ID: u32 = 1;

const FD_SIZE: usize = mem::size_of::<RawFd>();

/// A request being assembled for transmission to the compositor.
pub struct Msg {
    sender: u32,
    opcode: u16,
    body: Vec<u8>,
    fds: Vec<RawFd>,
}

impl Msg {
    pub fn new(sender: u32, opcode: u16) -> Self {
        Msg {
            sender,
            opcode,
            body: Vec::new(),
            fds: Vec::new(),
        }
    }

    pub fn uint(mut self, v: u32) -> Self {
        self.body.extend_from_slice(&v.to_ne_bytes());
        self
    }

    /// An object reference argument; `0` encodes a null object.
    pub fn object(self, id: u32) -> Self {
        self.uint(id)
    }

    /// A `new_id` argument for an object the client is creating.
    pub fn new_id(self, id: u32) -> Self {
        self.uint(id)
    }

    pub fn string(mut self, s: &str) -> Self {
        let bytes = s.as_bytes();
        // Length is prefixed and includes the trailing NUL terminator.
        let len = bytes.len() + 1;
        self.body.extend_from_slice(&(len as u32).to_ne_bytes());
        self.body.extend_from_slice(bytes);
        self.body.push(0);
        while !self.body.len().is_multiple_of(4) {
            self.body.push(0);
        }
        self
    }

    /// Attach a file descriptor; it is sent in ancillary data, not the body.
    pub fn fd(mut self, fd: RawFd) -> Self {
        self.fds.push(fd);
        self
    }
}

/// A decoded event received from the compositor.
pub struct Message {
    pub sender: u32,
    pub opcode: u16,
    pub args: Vec<u8>,
}

/// Sequentially decodes the argument bytes of a received [`Message`].
pub struct ArgReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ArgReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        ArgReader { buf, pos: 0 }
    }

    pub fn uint(&mut self) -> u32 {
        let v = u32::from_ne_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        v
    }

    /// An object reference; `0` means null.
    pub fn object(&mut self) -> u32 {
        self.uint()
    }

    /// A `new_id` the compositor has allocated (server-side ids are high).
    pub fn new_id(&mut self) -> u32 {
        self.uint()
    }

    /// A string argument; returns `None` for the null string (length 0).
    pub fn string(&mut self) -> Option<String> {
        let len = self.uint() as usize;
        if len == 0 {
            return None;
        }
        // `len` counts the NUL terminator; the text is the first len-1 bytes.
        let s = String::from_utf8_lossy(&self.buf[self.pos..self.pos + len - 1]).into_owned();
        let padded = (len + 3) & !3;
        self.pos += padded;
        Some(s)
    }
}

/// A live connection to the Wayland compositor.
pub struct Connection {
    stream: UnixStream,
    read_buf: Vec<u8>,
    fds_in: VecDeque<OwnedFd>,
    next_id: u32,
}

impl Connection {
    /// Connect to the compositor.
    ///
    /// Resolution order mirrors libwayland: an inherited `WAYLAND_SOCKET` fd
    /// takes priority (used inside sandboxes), then `display` / `WAYLAND_DISPLAY`
    /// resolved against `XDG_RUNTIME_DIR` (an absolute display name is used
    /// verbatim), defaulting to `wayland-0`.
    pub fn connect(display: Option<&str>) -> io::Result<Connection> {
        if let Ok(s) = env::var("WAYLAND_SOCKET")
            && let Ok(fd) = s.parse::<RawFd>()
        {
            // SAFETY: the parent handed us ownership of this fd.
            let stream = unsafe { UnixStream::from_raw_fd(fd) };
            unsafe { env::remove_var("WAYLAND_SOCKET") };
            return Ok(Connection::new(stream));
        }

        let name = display
            .map(str::to_string)
            .or_else(|| env::var("WAYLAND_DISPLAY").ok())
            .unwrap_or_else(|| "wayland-0".to_string());

        let path = if name.starts_with('/') {
            PathBuf::from(name)
        } else {
            let xdg = env::var_os("XDG_RUNTIME_DIR")
                .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
            let mut p = PathBuf::from(xdg);
            p.push(name);
            p
        };

        let stream = UnixStream::connect(&path).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "cannot connect to Wayland display at {}: {e}",
                    path.display()
                ),
            )
        })?;
        Ok(Connection::new(stream))
    }

    /// Wrap an already-connected stream. Used by tests to drive the client
    /// against an in-process mock compositor over a socketpair.
    #[cfg(test)]
    pub(crate) fn from_stream(stream: UnixStream) -> Connection {
        Connection::new(stream)
    }

    fn new(stream: UnixStream) -> Connection {
        Connection {
            stream,
            read_buf: Vec::new(),
            fds_in: VecDeque::new(),
            // Client object ids start at 2; id 1 is always wl_display.
            next_id: 2,
        }
    }

    /// Allocate a fresh client-side object id.
    pub fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Pull the next file descriptor delivered alongside events, if any.
    pub fn take_fd(&mut self) -> Option<OwnedFd> {
        self.fds_in.pop_front()
    }

    /// Serialize and send a request.
    pub fn send(&mut self, msg: Msg) -> io::Result<()> {
        let total = 8 + msg.body.len();
        if total > u16::MAX as usize {
            return Err(io::Error::new(ErrorKind::InvalidInput, "message too large"));
        }
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&msg.sender.to_ne_bytes());
        let word2 = ((total as u32) << 16) | (msg.opcode as u32);
        out.extend_from_slice(&word2.to_ne_bytes());
        out.extend_from_slice(&msg.body);
        self.send_bytes(&out, &msg.fds)
    }

    /// Read and decode the next event from the compositor.
    pub fn next_message(&mut self) -> io::Result<Message> {
        self.fill(8)?;
        let sender = u32::from_ne_bytes(self.read_buf[0..4].try_into().unwrap());
        let word2 = u32::from_ne_bytes(self.read_buf[4..8].try_into().unwrap());
        let opcode = (word2 & 0xffff) as u16;
        let size = (word2 >> 16) as usize;
        if size < 8 {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "malformed Wayland message header",
            ));
        }
        self.fill(size)?;
        let args = self.read_buf[8..size].to_vec();
        self.read_buf.drain(..size);
        Ok(Message {
            sender,
            opcode,
            args,
        })
    }

    /// Ensure at least `need` bytes are buffered, reading from the socket.
    fn fill(&mut self, need: usize) -> io::Result<()> {
        while self.read_buf.len() < need {
            self.recv()?;
        }
        Ok(())
    }

    /// One `recvmsg`, appending payload bytes and any passed fds to our buffers.
    fn recv(&mut self) -> io::Result<()> {
        const CAP: usize = 4096;
        let mut buf = [0u8; CAP];
        let mut cmsg = [0u8; 512];
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr() as *mut libc::c_void,
            iov_len: CAP,
        };
        // SAFETY: msghdr is plain data; we fully initialize the fields we use.
        let mut mhdr: libc::msghdr = unsafe { mem::zeroed() };
        mhdr.msg_iov = &mut iov;
        mhdr.msg_iovlen = 1;
        mhdr.msg_control = cmsg.as_mut_ptr() as *mut libc::c_void;
        mhdr.msg_controllen = cmsg.len() as _;

        let n =
            unsafe { libc::recvmsg(self.stream.as_raw_fd(), &mut mhdr, libc::MSG_CMSG_CLOEXEC) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == ErrorKind::Interrupted {
                return Ok(());
            }
            return Err(err);
        }
        if n == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "the Wayland connection was closed by the compositor",
            ));
        }

        // Harvest any file descriptors from SCM_RIGHTS control messages.
        unsafe {
            let mut hdr = libc::CMSG_FIRSTHDR(&mhdr);
            while !hdr.is_null() {
                if (*hdr).cmsg_level == libc::SOL_SOCKET && (*hdr).cmsg_type == libc::SCM_RIGHTS {
                    let data = libc::CMSG_DATA(hdr);
                    let payload = (*hdr).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                    let count = payload / FD_SIZE;
                    for i in 0..count {
                        let mut fd: RawFd = 0;
                        ptr::copy_nonoverlapping(
                            data.add(i * FD_SIZE),
                            &mut fd as *mut RawFd as *mut u8,
                            FD_SIZE,
                        );
                        self.fds_in.push_back(OwnedFd::from_raw_fd(fd));
                    }
                }
                hdr = libc::CMSG_NXTHDR(&mhdr, hdr);
            }
        }

        self.read_buf.extend_from_slice(&buf[..n as usize]);
        Ok(())
    }

    /// Send raw bytes, attaching `fds` to the first `sendmsg` via SCM_RIGHTS.
    fn send_bytes(&mut self, bytes: &[u8], fds: &[RawFd]) -> io::Result<()> {
        let cmsg_space = if fds.is_empty() {
            0
        } else {
            unsafe { libc::CMSG_SPACE((fds.len() * FD_SIZE) as u32) as usize }
        };
        let mut cmsg = vec![0u8; cmsg_space];

        let mut sent = 0;
        while sent < bytes.len() {
            let mut iov = libc::iovec {
                iov_base: bytes[sent..].as_ptr() as *mut libc::c_void,
                iov_len: bytes.len() - sent,
            };
            // SAFETY: msghdr is plain data; we initialize the fields we use.
            let mut mhdr: libc::msghdr = unsafe { mem::zeroed() };
            mhdr.msg_iov = &mut iov;
            mhdr.msg_iovlen = 1;

            // Descriptors ride along only with the first chunk.
            if sent == 0 && !fds.is_empty() {
                mhdr.msg_control = cmsg.as_mut_ptr() as *mut libc::c_void;
                mhdr.msg_controllen = cmsg.len() as _;
                unsafe {
                    let hdr = libc::CMSG_FIRSTHDR(&mhdr);
                    (*hdr).cmsg_level = libc::SOL_SOCKET;
                    (*hdr).cmsg_type = libc::SCM_RIGHTS;
                    (*hdr).cmsg_len = libc::CMSG_LEN((fds.len() * FD_SIZE) as u32) as _;
                    let data = libc::CMSG_DATA(hdr);
                    for (i, &fd) in fds.iter().enumerate() {
                        ptr::copy_nonoverlapping(
                            &fd as *const RawFd as *const u8,
                            data.add(i * FD_SIZE),
                            FD_SIZE,
                        );
                    }
                }
            }

            let n = unsafe { libc::sendmsg(self.stream.as_raw_fd(), &mhdr, libc::MSG_NOSIGNAL) };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            sent += n as usize;
        }
        Ok(())
    }
}

/// Create a pipe, returning `(read_end, write_end)`, both close-on-exec.
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as RawFd; 2];
    let r = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: pipe2 produced two fresh, owned descriptors.
    unsafe { Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}
