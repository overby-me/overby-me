// Internal harness: connect to a running PipeWire daemon, send a single
// Core.Hello and dump the first few server responses (header + decoded
// args) as text. Used by the daemon-interop nix test.

use crate::pipewire_lib::proto_native::{HDR_SIZE, decode, encode_call};
use crate::spa::pod::types::Value;
use std::env;
use std::io::{self, IoSlice, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const CORE_OP_HELLO: u8 = 1;
const CORE_OP_GET_REGISTRY: u8 = 5;
const CLIENT_OP_UPDATE_PROPERTIES: u8 = 2;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("proto-probe");

    // Arguments:
    //   <0|none> [max_messages]
    let max_messages: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);

    let socket = match resolve_socket() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{argv0}: socket resolve: {e}");
            return 2;
        }
    };

    eprintln!("{argv0}: connecting to {}", socket.display());
    let stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{argv0}: connect: {e}");
            return 1;
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));

    if let Err(e) = handshake(&stream) {
        eprintln!("{argv0}: handshake: {e}");
        return 1;
    }

    if let Err(e) = drain(&stream, max_messages) {
        eprintln!("{argv0}: drain: {e}");
        return 1;
    }
    0
}

fn resolve_socket() -> io::Result<PathBuf> {
    // Mirror `pw-cli`'s default: $PIPEWIRE_REMOTE > $XDG_RUNTIME_DIR/$PIPEWIRE_CORE.
    if let Ok(remote) = env::var("PIPEWIRE_REMOTE")
        && remote.starts_with('/')
    {
        return Ok(PathBuf::from(remote));
    }
    let runtime = env::var("XDG_RUNTIME_DIR")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR unset"))?;
    let core_name = env::var("PIPEWIRE_CORE").unwrap_or_else(|_| "pipewire-0".into());
    Ok(PathBuf::from(runtime).join(core_name))
}

fn handshake(mut stream: &UnixStream) -> io::Result<()> {
    // Core.Hello(version=3) on the core proxy (id=0).
    let hello = encode_call(0, CORE_OP_HELLO, 0, &[Value::Int(3)]);
    stream.write_all(&hello.bytes)?;

    // Client.UpdateProperties on the client proxy (id=1) — a typical
    // first-message follow-up so the daemon registers our properties.
    let props = encode_call(
        1,
        CLIENT_OP_UPDATE_PROPERTIES,
        1,
        &[Value::Struct(vec![
            // dict: count, then alternating key/value strings
            Value::Int(1),
            Value::String("application.name".into()),
            Value::String("rust-pipewire-probe".into()),
        ])],
    );
    stream.write_all(&props.bytes)?;

    // Core.GetRegistry(version=3, new_id=2) — opcode 5 in C
    // protocol-native.c. Body: (i32 version, i32 new_id).
    let get_reg = encode_call(0, CORE_OP_GET_REGISTRY, 2, &[Value::Int(3), Value::Int(2)]);
    stream.write_all(&get_reg.bytes)?;

    let _ = IoSlice::new(&[]); // anchor for io::Write trait
    Ok(())
}

fn drain(mut stream: &UnixStream, max: usize) -> io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 65536];
    let mut read_count = 0;
    let mut printed = 0;

    while printed < max {
        // Read at least HDR_SIZE more bytes than we have.
        if buf.len() < HDR_SIZE {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    read_count += 1;
                    let _ = read_count;
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(e) => return Err(e),
            }
            continue;
        }

        match decode(&buf) {
            Ok((d, used)) => {
                println!(
                    "msg id={} opcode={} seq={} n_fds={} args={}",
                    d.id,
                    d.opcode,
                    d.seq,
                    d.n_fds,
                    summarize_args(&d.args)
                );
                buf.drain(..used);
                printed += 1;
            }
            Err(_) => {
                // Need more bytes.
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    Err(e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }

    Ok(())
}

fn summarize_args(args: &[Value]) -> String {
    let mut out = String::from("[");
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&summarize(a));
    }
    out.push(']');
    out
}

fn summarize(v: &Value) -> String {
    match v {
        Value::None => "None".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Long(n) => n.to_string(),
        Value::Id(n) => format!("Id({n})"),
        Value::Float(n) => format!("{n}"),
        Value::Double(n) => format!("{n}"),
        Value::String(s) => format!("{s:?}"),
        Value::Bytes(b) => format!("Bytes({})", b.len()),
        Value::Rectangle(r) => format!("{}x{}", r.width, r.height),
        Value::Fraction(f) => format!("{}/{}", f.num, f.denom),
        Value::Bitmap(b) => format!("Bitmap({})", b.len()),
        Value::Fd(n) => format!("Fd({n})"),
        Value::Pointer { ptype, .. } => format!("Pointer({ptype})"),
        Value::Array { elements, .. } => format!("Array({})", elements.len()),
        Value::Struct(items) => format!("Struct({})", items.len()),
        Value::Object {
            object_type,
            properties,
            ..
        } => format!("Object({object_type}, {} props)", properties.len()),
        Value::Sequence { controls, .. } => format!("Sequence({})", controls.len()),
        Value::Choice { elements, .. } => format!("Choice({})", elements.len()),
    }
}
