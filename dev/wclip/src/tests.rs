//! Roundtrip tests against an in-process mock compositor.
//!
//! A real data-control-capable compositor isn't available in CI, so these tests
//! stand up a minimal one on the server end of a `socketpair` and drive the
//! real client code against it. This exercises the genuine wire encoding and
//! decoding plus SCM_RIGHTS file-descriptor passing for both directions.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;

use crate::proto::{self, State};
use crate::wire::{ArgReader, Connection, DISPLAY_ID, Msg, pipe};

/// A connected pair of unix-domain stream sockets.
fn socketpair() -> (UnixStream, UnixStream) {
    let mut fds = [0 as RawFd; 2];
    let r = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
    assert_eq!(r, 0, "socketpair failed");
    // SAFETY: socketpair produced two fresh, owned descriptors.
    unsafe {
        (
            UnixStream::from_raw_fd(fds[0]),
            UnixStream::from_raw_fd(fds[1]),
        )
    }
}

/// Emit the two globals a clipboard client needs: a data-control manager and a
/// seat. Returns the registry id learned from the `get_registry` request.
fn advertise_globals(server: &mut Connection, args: &[u8]) -> u32 {
    let registry = ArgReader::new(args).new_id();
    server
        .send(
            Msg::new(registry, 0)
                .uint(1)
                .string("ext_data_control_manager_v1")
                .uint(1),
        )
        .unwrap();
    server
        .send(Msg::new(registry, 0).uint(2).string("wl_seat").uint(1))
        .unwrap();
    registry
}

/// Reply to a `wl_display.sync` with the matching `wl_callback.done`.
fn ack_sync(server: &mut Connection, args: &[u8]) {
    let callback = ArgReader::new(args).new_id();
    server.send(Msg::new(callback, 0).uint(0)).unwrap();
}

#[test]
fn paste_roundtrip_delivers_selection() {
    let (client, server) = socketpair();
    let payload = b"clipboard contents\n".to_vec();
    let expected = payload.clone();

    let handle = thread::spawn(move || {
        let mut s = Connection::from_stream(server);
        let (mut registry, mut manager, mut offer) = (0u32, 0u32, 0u32);
        loop {
            let Ok(m) = s.next_message() else { return };
            let mut r = ArgReader::new(&m.args);
            match (m.sender, m.opcode) {
                (DISPLAY_ID, 1) => registry = advertise_globals(&mut s, &m.args),
                (DISPLAY_ID, 0) => ack_sync(&mut s, &m.args),
                (s_id, 0) if s_id == registry => {
                    // bind: name, interface, version, new_id
                    let _name = r.uint();
                    let iface = r.string().unwrap();
                    let _ver = r.uint();
                    let id = r.new_id();
                    if iface != "wl_seat" {
                        manager = id;
                    }
                }
                (s_id, 1) if s_id == manager => {
                    // get_data_device: announce a current selection.
                    let device = r.new_id();
                    offer = 0xff00_0001;
                    s.send(Msg::new(device, 0).new_id(offer)).unwrap(); // data_offer
                    s.send(Msg::new(offer, 0).string("text/plain;charset=utf-8"))
                        .unwrap(); // offer
                    s.send(Msg::new(offer, 0).string("text/plain")).unwrap();
                    s.send(Msg::new(device, 1).object(offer)).unwrap(); // selection
                }
                (s_id, 0) if s_id == offer => {
                    // receive: write the payload into the passed fd.
                    let _mime = r.string();
                    let fd = s.take_fd().expect("receive carried no fd");
                    File::from(fd).write_all(&payload).unwrap();
                    return;
                }
                _ => {}
            }
        }
    });

    let mut conn = Connection::from_stream(client);
    let mut st = State::new();
    proto::setup(&mut conn, &mut st).unwrap();
    assert!(st.primary_supported(), "ext-data-control offers primary");
    proto::roundtrip(&mut conn, &mut st).unwrap();

    let offer = st.selection.expect("a selection should be present");
    assert_eq!(
        st.offer_mimes(offer),
        ["text/plain;charset=utf-8", "text/plain"]
    );

    let data = proto::receive(&mut conn, offer, "text/plain;charset=utf-8").unwrap();
    assert_eq!(data, expected);
    handle.join().unwrap();
}

#[test]
fn copy_roundtrip_serves_data() {
    let (client, server) = socketpair();
    let (tx, rx) = mpsc::channel();

    let handle = thread::spawn(move || {
        let mut s = Connection::from_stream(server);
        let (mut registry, mut manager, mut source) = (0u32, 0u32, 0u32);
        let mut syncs = 0;
        loop {
            let Ok(m) = s.next_message() else { return };
            let mut r = ArgReader::new(&m.args);
            match (m.sender, m.opcode) {
                (DISPLAY_ID, 1) => registry = advertise_globals(&mut s, &m.args),
                (DISPLAY_ID, 0) => {
                    ack_sync(&mut s, &m.args);
                    syncs += 1;
                    // After setup + set_selection complete, the client is about
                    // to serve. Act as a pasting client: ask the source to send.
                    if syncs == 2 {
                        let (read_end, write_end) = pipe().unwrap();
                        s.send(
                            Msg::new(source, 0)
                                .string("text/plain")
                                .fd(write_end.as_raw_fd()),
                        )
                        .unwrap(); // source.send event
                        drop(write_end);
                        let mut buf = Vec::new();
                        File::from(read_end).read_to_end(&mut buf).unwrap();
                        tx.send(buf).unwrap();
                        s.send(Msg::new(source, 1)).unwrap(); // cancelled → stop serving
                        return;
                    }
                }
                (s_id, 0) if s_id == registry => {
                    let _name = r.uint();
                    let iface = r.string().unwrap();
                    let _ver = r.uint();
                    let id = r.new_id();
                    if iface != "wl_seat" {
                        manager = id;
                    }
                }
                (s_id, 1) if s_id == manager => {
                    let _device = r.new_id();
                }
                (s_id, 0) if s_id == manager => source = r.new_id(), // create_data_source
                _ => {}
            }
        }
    });

    let mut conn = Connection::from_stream(client);
    let mut st = State::new();
    proto::setup(&mut conn, &mut st).unwrap();

    let mimes = vec!["text/plain".to_string()];
    proto::set_selection(&mut conn, &mut st, &mimes, false).unwrap();
    assert!(!st.cancelled);

    st.data = b"hello from wclip".to_vec();
    proto::serve(&mut conn, &mut st).unwrap();

    let received = rx.recv().unwrap();
    assert_eq!(received, b"hello from wclip");
    handle.join().unwrap();
}
