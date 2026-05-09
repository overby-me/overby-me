// Higher-level protocol-native client.
//
// Wraps a Unix-stream connection to a PipeWire daemon, owns the read/write
// buffer, and exposes typed helpers for sending common methods and decoding
// inbound events into structured forms (CoreInfo, RegistryGlobal, ...).
//
// This is the layer the tools (pw-cli, pw-dump, pw-mon) build on top of.

use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::pipewire_lib::interfaces::{
    self, ID_CLIENT, ID_CORE, VERSION_CORE, VERSION_REGISTRY,
    client_method, core_method,
};
use crate::pipewire_lib::proto_native::{Decoded, HDR_SIZE, Message, decode, encode_call};
use crate::spa::pod::types::Value;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Decode(String),
    UnexpectedShape(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Decode(s) => write!(f, "decode: {s}"),
            Error::UnexpectedShape(s) => write!(f, "unexpected wire shape: {s}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// One key=value pair from a PipeWire dict.
#[derive(Debug, Clone, PartialEq)]
pub struct DictItem {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreInfo {
    pub id: u32,
    pub cookie: u32,
    pub user_name: String,
    pub host_name: String,
    pub version: String,
    pub name: String,
    pub change_mask: i64,
    pub props: Vec<DictItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegistryGlobal {
    pub id: u32,
    pub permissions: u32,
    pub interface: String,
    pub version: u32,
    pub props: Vec<DictItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleInfo {
    pub id: u32,
    pub name: String,
    pub filename: String,
    pub args: String,
    pub change_mask: i64,
    pub props: Vec<DictItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FactoryInfo {
    pub id: u32,
    pub name: String,
    pub object_type: String,
    pub version: u32,
    pub change_mask: i64,
    pub props: Vec<DictItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfo {
    pub id: u32,
    pub change_mask: i64,
    pub props: Vec<DictItem>,
}

/// One entry of a `params` array (id + flags) on a Node, Port, or Device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamInfo {
    pub id: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeInfo {
    pub id: u32,
    pub max_input_ports: u32,
    pub max_output_ports: u32,
    pub change_mask: i64,
    pub n_input_ports: u32,
    pub n_output_ports: u32,
    pub state: u32,
    pub error: String,
    pub props: Vec<DictItem>,
    pub params: Vec<ParamInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PortInfo {
    pub id: u32,
    pub direction: u32, // 0=input, 1=output
    pub change_mask: i64,
    pub props: Vec<DictItem>,
    pub params: Vec<ParamInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub id: u32,
    pub change_mask: i64,
    pub props: Vec<DictItem>,
    pub params: Vec<ParamInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkInfo {
    pub id: u32,
    pub output_node_id: u32,
    pub output_port_id: u32,
    pub input_node_id: u32,
    pub input_port_id: u32,
    pub change_mask: i64,
    pub state: i32,
    pub error: String,
    /// Negotiated format POD; `None` when no format yet (unlinked / init).
    pub format: Option<Value>,
    pub props: Vec<DictItem>,
}

/// Resolve `$XDG_RUNTIME_DIR/$PIPEWIRE_CORE` (with the standard fallbacks
/// used by `pw-cli`).
pub fn resolve_socket() -> io::Result<PathBuf> {
    if let Ok(remote) = env::var("PIPEWIRE_REMOTE") {
        if remote.starts_with('/') {
            return Ok(PathBuf::from(remote));
        }
    }
    let runtime = env::var("XDG_RUNTIME_DIR")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR unset"))?;
    let core_name = env::var("PIPEWIRE_CORE").unwrap_or_else(|_| "pipewire-0".into());
    Ok(PathBuf::from(runtime).join(core_name))
}

/// Buffered reader over a UnixStream; decodes one protocol message at a time.
pub struct Client {
    stream: UnixStream,
    rx: Vec<u8>,
    chunk: [u8; 65536],
    next_id: u32,
    next_seq: u32,
    timeout: Duration,
}

impl Client {
    pub fn connect_default() -> Result<Self, Error> {
        let path = resolve_socket()?;
        Self::connect_path(&path)
    }

    pub fn connect_path(path: &std::path::Path) -> Result<Self, Error> {
        let stream = UnixStream::connect(path)?;
        // Generous timeout: the Nix sandbox is slower than real hardware,
        // and the daemon may take a moment to satisfy a Bind that triggers
        // module/factory introspection.
        let timeout = Duration::from_secs(5);
        stream.set_read_timeout(Some(timeout))?;
        Ok(Client {
            stream,
            rx: Vec::with_capacity(64 * 1024),
            chunk: [0u8; 65536],
            // First locally-allocated proxy id. ids 0 and 1 are reserved
            // for Core and Client by the protocol.
            next_id: 2,
            next_seq: 0,
            timeout,
        })
    }

    pub fn set_timeout(&mut self, t: Duration) -> io::Result<()> {
        self.timeout = t;
        self.stream.set_read_timeout(Some(t))
    }

    fn alloc_seq(&mut self) -> u32 {
        let s = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        s
    }

    /// Allocate a new client-side proxy id.
    pub fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    fn send(&mut self, m: Message) -> io::Result<()> {
        self.stream.write_all(&m.bytes)
    }

    /// Send the standard handshake: Hello, UpdateProperties, GetRegistry.
    /// Returns the registry proxy id.
    pub fn handshake(&mut self, app_name: &str) -> Result<u32, Error> {
        let seq = self.alloc_seq();
        self.send(encode_call(
            ID_CORE,
            core_method::HELLO,
            seq,
            &[Value::Int(VERSION_CORE as i32)],
        ))?;

        // The body of UpdateProperties is `Struct { Struct { Int(n_items),
        // String key, String val, ... } }`. encode_call already wraps args
        // in an outer Struct, so `args` itself must be the inner dict struct.
        let dict = build_dict(&[("application.name", app_name)]);
        let seq = self.alloc_seq();
        self.send(encode_call(
            ID_CLIENT,
            client_method::UPDATE_PROPERTIES,
            seq,
            &[dict],
        ))?;

        let registry_id = self.alloc_id();
        let seq = self.alloc_seq();
        self.send(encode_call(
            ID_CORE,
            core_method::GET_REGISTRY,
            seq,
            &[
                Value::Int(VERSION_REGISTRY as i32),
                Value::Int(registry_id as i32),
            ],
        ))?;
        Ok(registry_id)
    }

    /// Send `Registry.Bind` on the registry proxy with id `registry_id`.
    /// Returns the freshly-allocated proxy id which the server will use as
    /// the source of subsequent events for the bound object.
    pub fn registry_bind(
        &mut self,
        registry_id: u32,
        global_id: u32,
        interface: &str,
        version: u32,
    ) -> Result<u32, Error> {
        let new_id = self.alloc_id();
        let seq = self.alloc_seq();
        self.send(encode_call(
            registry_id,
            interfaces::registry_method::BIND,
            seq,
            &[
                Value::Int(global_id as i32),
                Value::String(interface.to_string()),
                Value::Int(version as i32),
                Value::Int(new_id as i32),
            ],
        ))?;
        Ok(new_id)
    }

    /// Send Core.Sync(id, seq) — server replies with Core.Done(id, seq) once
    /// it has processed every prior message. We pick a unique seq and return
    /// it so callers can wait for the matching Done.
    pub fn sync(&mut self, target_id: u32) -> Result<u32, Error> {
        let seq = self.alloc_seq();
        self.send(encode_call(
            ID_CORE,
            core_method::SYNC,
            seq,
            &[Value::Int(target_id as i32), Value::Int(seq as i32)],
        ))?;
        Ok(seq)
    }

    /// Read one decoded protocol message, blocking up to `timeout` total.
    /// Returns `Ok(None)` on EOF / timeout with no message in flight.
    pub fn read_message(&mut self) -> Result<Option<Decoded>, Error> {
        let deadline = Instant::now() + self.timeout;
        loop {
            // Try decoding what we have first.
            if self.rx.len() >= HDR_SIZE
                && let Ok((d, used)) = decode(&self.rx)
            {
                self.rx.drain(..used);
                return Ok(Some(d));
            }
            // Need more bytes.
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            self.stream.set_read_timeout(Some(deadline - now))?;
            match self.stream.read(&mut self.chunk) {
                Ok(0) => return Ok(None),
                Ok(n) => self.rx.extend_from_slice(&self.chunk[..n]),
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(Error::Io(e)),
            }
        }
    }

    /// Read messages until `pred(msg) -> Some(out)` succeeds, or no more
    /// messages arrive within the timeout. Useful for "wait for Done".
    pub fn drain_until<T>(
        &mut self,
        mut pred: impl FnMut(&Decoded) -> Option<T>,
        mut on_other: impl FnMut(&Decoded),
    ) -> Result<Option<T>, Error> {
        loop {
            match self.read_message()? {
                Some(m) => {
                    if let Some(t) = pred(&m) {
                        return Ok(Some(t));
                    } else {
                        on_other(&m);
                    }
                }
                None => return Ok(None),
            }
        }
    }
}

/// Construct the inner-Struct dict POD from a flat `[(k, v)]` list.
fn build_dict(items: &[(&str, &str)]) -> Value {
    let mut struct_items = Vec::with_capacity(1 + items.len() * 2);
    struct_items.push(Value::Int(items.len() as i32));
    for (k, v) in items {
        struct_items.push(Value::String((*k).to_string()));
        struct_items.push(Value::String((*v).to_string()));
    }
    Value::Struct(struct_items)
}

/// Decode the inner dict struct (`{ Int n_items, key, val, ... }`).
fn decode_dict_struct(v: &Value) -> Result<Vec<DictItem>, Error> {
    let items = match v {
        Value::Struct(items) => items,
        _ => return Err(Error::UnexpectedShape("dict: not a Struct")),
    };
    let mut iter = items.iter();
    let n = match iter.next() {
        Some(Value::Int(n)) if *n >= 0 => *n as usize,
        _ => return Err(Error::UnexpectedShape("dict: n_items not Int>=0")),
    };
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let k = match iter.next() {
            Some(Value::String(s)) => s.clone(),
            _ => return Err(Error::UnexpectedShape("dict: key not String")),
        };
        let val = match iter.next() {
            Some(Value::String(s)) => s.clone(),
            // Pointer values get cleaned up by the daemon to "" — but the C
            // parser also accepts other shapes by ignoring them; play it safe.
            _ => return Err(Error::UnexpectedShape("dict: value not String")),
        };
        out.push(DictItem { key: k, value: val });
    }
    Ok(out)
}

/// Decode `Core.Info` (id=0, opcode=0).
pub fn decode_core_info(args: &[Value]) -> Result<CoreInfo, Error> {
    if args.len() < 8 {
        return Err(Error::UnexpectedShape("Core.Info: not enough fields"));
    }
    let id = expect_int(&args[0], "Core.Info.id")? as u32;
    let cookie = expect_int(&args[1], "Core.Info.cookie")? as u32;
    let user_name = expect_string(&args[2], "Core.Info.user_name")?;
    let host_name = expect_string(&args[3], "Core.Info.host_name")?;
    let version = expect_string(&args[4], "Core.Info.version")?;
    let name = expect_string(&args[5], "Core.Info.name")?;
    let change_mask = expect_long(&args[6], "Core.Info.change_mask")?;
    let props = decode_dict_struct(&args[7])?;
    Ok(CoreInfo {
        id,
        cookie,
        user_name,
        host_name,
        version,
        name,
        change_mask,
        props,
    })
}

/// Decode `Module.Info` (event opcode 0 on the module proxy):
/// `Struct { Int id, String name, String filename, String args, Long
/// change_mask, Struct dict }`.
pub fn decode_module_info(args: &[Value]) -> Result<ModuleInfo, Error> {
    if args.len() < 6 {
        return Err(Error::UnexpectedShape("Module.Info: not enough fields"));
    }
    Ok(ModuleInfo {
        id: expect_int(&args[0], "Module.Info.id")? as u32,
        name: expect_string(&args[1], "Module.Info.name")?,
        filename: expect_string(&args[2], "Module.Info.filename")?,
        args: expect_string(&args[3], "Module.Info.args")?,
        change_mask: expect_long(&args[4], "Module.Info.change_mask")?,
        props: decode_dict_struct(&args[5])?,
    })
}

/// Decode `Factory.Info` (event opcode 0):
/// `Struct { Int id, String name, String object_type, Int version, Long
/// change_mask, Struct dict }`.
pub fn decode_factory_info(args: &[Value]) -> Result<FactoryInfo, Error> {
    if args.len() < 6 {
        return Err(Error::UnexpectedShape("Factory.Info: not enough fields"));
    }
    Ok(FactoryInfo {
        id: expect_int(&args[0], "Factory.Info.id")? as u32,
        name: expect_string(&args[1], "Factory.Info.name")?,
        object_type: expect_string(&args[2], "Factory.Info.object_type")?,
        version: expect_int(&args[3], "Factory.Info.version")? as u32,
        change_mask: expect_long(&args[4], "Factory.Info.change_mask")?,
        props: decode_dict_struct(&args[5])?,
    })
}

/// Decode the inner params struct (`{ Int n_params, Id id1, Int flags1,
/// Id id2, Int flags2, ... }`).
fn decode_params_struct(v: &Value) -> Result<Vec<ParamInfo>, Error> {
    let items = match v {
        Value::Struct(items) => items,
        _ => return Err(Error::UnexpectedShape("params: not a Struct")),
    };
    let mut iter = items.iter();
    let n = match iter.next() {
        Some(Value::Int(n)) if *n >= 0 => *n as usize,
        _ => {
            return Err(Error::UnexpectedShape(
                "params: n_params not Int>=0",
            ))
        }
    };
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let id = match iter.next() {
            Some(Value::Id(id)) => *id,
            _ => return Err(Error::UnexpectedShape("params: id not Id")),
        };
        let flags = match iter.next() {
            Some(Value::Int(f)) => *f as u32,
            _ => return Err(Error::UnexpectedShape("params: flags not Int")),
        };
        out.push(ParamInfo { id, flags });
    }
    Ok(out)
}

/// Decode `Node.Info`:
/// `Struct { Int id, Int max_in, Int max_out, Long change_mask, Int n_in,
/// Int n_out, Id state, String error, Struct dict, Struct params }`.
pub fn decode_node_info(args: &[Value]) -> Result<NodeInfo, Error> {
    if args.len() < 10 {
        return Err(Error::UnexpectedShape("Node.Info: not enough fields"));
    }
    Ok(NodeInfo {
        id: expect_int(&args[0], "Node.Info.id")? as u32,
        max_input_ports: expect_int(&args[1], "Node.Info.max_input_ports")? as u32,
        max_output_ports: expect_int(&args[2], "Node.Info.max_output_ports")? as u32,
        change_mask: expect_long(&args[3], "Node.Info.change_mask")?,
        n_input_ports: expect_int(&args[4], "Node.Info.n_input_ports")? as u32,
        n_output_ports: expect_int(&args[5], "Node.Info.n_output_ports")? as u32,
        state: match &args[6] {
            Value::Id(s) => *s,
            _ => {
                return Err(Error::Decode(
                    "Node.Info.state: expected Id".into(),
                ))
            }
        },
        error: expect_string(&args[7], "Node.Info.error")?,
        props: decode_dict_struct(&args[8])?,
        params: decode_params_struct(&args[9])?,
    })
}

/// Decode `Port.Info`:
/// `Struct { Int id, Int direction, Long change_mask, Struct dict, Struct params }`.
pub fn decode_port_info(args: &[Value]) -> Result<PortInfo, Error> {
    if args.len() < 5 {
        return Err(Error::UnexpectedShape("Port.Info: not enough fields"));
    }
    Ok(PortInfo {
        id: expect_int(&args[0], "Port.Info.id")? as u32,
        direction: expect_int(&args[1], "Port.Info.direction")? as u32,
        change_mask: expect_long(&args[2], "Port.Info.change_mask")?,
        props: decode_dict_struct(&args[3])?,
        params: decode_params_struct(&args[4])?,
    })
}

/// Decode `Link.Info`:
/// `Struct { Int id, Int out_node, Int out_port, Int in_node, Int in_port,
/// Long change_mask, Int state, String error, Pod format, Struct dict }`.
///
/// `format` may be a None POD when the link hasn't negotiated a format.
pub fn decode_link_info(args: &[Value]) -> Result<LinkInfo, Error> {
    if args.len() < 10 {
        return Err(Error::UnexpectedShape("Link.Info: not enough fields"));
    }
    let format = match &args[8] {
        Value::None => None,
        other => Some(other.clone()),
    };
    Ok(LinkInfo {
        id: expect_int(&args[0], "Link.Info.id")? as u32,
        output_node_id: expect_int(&args[1], "Link.Info.output_node_id")? as u32,
        output_port_id: expect_int(&args[2], "Link.Info.output_port_id")? as u32,
        input_node_id: expect_int(&args[3], "Link.Info.input_node_id")? as u32,
        input_port_id: expect_int(&args[4], "Link.Info.input_port_id")? as u32,
        change_mask: expect_long(&args[5], "Link.Info.change_mask")?,
        state: expect_int(&args[6], "Link.Info.state")?,
        error: expect_string(&args[7], "Link.Info.error")?,
        format,
        props: decode_dict_struct(&args[9])?,
    })
}

/// Decode `Device.Info`:
/// `Struct { Int id, Long change_mask, Struct dict, Struct params }`.
pub fn decode_device_info(args: &[Value]) -> Result<DeviceInfo, Error> {
    if args.len() < 4 {
        return Err(Error::UnexpectedShape("Device.Info: not enough fields"));
    }
    Ok(DeviceInfo {
        id: expect_int(&args[0], "Device.Info.id")? as u32,
        change_mask: expect_long(&args[1], "Device.Info.change_mask")?,
        props: decode_dict_struct(&args[2])?,
        params: decode_params_struct(&args[3])?,
    })
}

/// Decode `Client.Info` (event opcode 0):
/// `Struct { Int id, Long change_mask, Struct dict }`.
pub fn decode_client_info(args: &[Value]) -> Result<ClientInfo, Error> {
    if args.len() < 3 {
        return Err(Error::UnexpectedShape("Client.Info: not enough fields"));
    }
    Ok(ClientInfo {
        id: expect_int(&args[0], "Client.Info.id")? as u32,
        change_mask: expect_long(&args[1], "Client.Info.change_mask")?,
        props: decode_dict_struct(&args[2])?,
    })
}

/// Decode `Registry.Global` (id=registry_proxy, opcode=0).
pub fn decode_registry_global(args: &[Value]) -> Result<RegistryGlobal, Error> {
    if args.len() < 5 {
        return Err(Error::UnexpectedShape(
            "Registry.Global: not enough fields",
        ));
    }
    let id = expect_int(&args[0], "Registry.Global.id")? as u32;
    let permissions = expect_int(&args[1], "Registry.Global.permissions")? as u32;
    let interface = expect_string(&args[2], "Registry.Global.type")?;
    let version = expect_int(&args[3], "Registry.Global.version")? as u32;
    let props = decode_dict_struct(&args[4])?;
    Ok(RegistryGlobal {
        id,
        permissions,
        interface,
        version,
        props,
    })
}

/// Decode `Registry.GlobalRemove` (id=registry_proxy, opcode=1).
pub fn decode_registry_global_remove(args: &[Value]) -> Result<u32, Error> {
    if args.is_empty() {
        return Err(Error::UnexpectedShape(
            "Registry.GlobalRemove: empty args",
        ));
    }
    let id = expect_int(&args[0], "Registry.GlobalRemove.id")? as u32;
    Ok(id)
}

/// Decode `Core.Done(id, seq)` (id=0, opcode=1).
pub fn decode_core_done(args: &[Value]) -> Result<(u32, u32), Error> {
    if args.len() < 2 {
        return Err(Error::UnexpectedShape("Core.Done: not enough fields"));
    }
    let id = expect_int(&args[0], "Core.Done.id")? as u32;
    let seq = expect_int(&args[1], "Core.Done.seq")? as u32;
    Ok((id, seq))
}

/// Decode `Core.Ping(id, seq)` so the client can reply with Core.Pong.
pub fn decode_core_ping(args: &[Value]) -> Result<(u32, u32), Error> {
    if args.len() < 2 {
        return Err(Error::UnexpectedShape("Core.Ping: not enough fields"));
    }
    Ok((
        expect_int(&args[0], "Core.Ping.id")? as u32,
        expect_int(&args[1], "Core.Ping.seq")? as u32,
    ))
}

/// Decode `Core.Error(id, seq, res, message)`.
pub fn decode_core_error(args: &[Value]) -> Result<(u32, i32, i32, String), Error> {
    if args.len() < 4 {
        return Err(Error::UnexpectedShape("Core.Error: not enough fields"));
    }
    let id = expect_int(&args[0], "Core.Error.id")? as u32;
    let seq = expect_int(&args[1], "Core.Error.seq")?;
    let res = expect_int(&args[2], "Core.Error.res")?;
    let message = expect_string(&args[3], "Core.Error.message")?;
    Ok((id, seq, res, message))
}

fn expect_int(v: &Value, ctx: &'static str) -> Result<i32, Error> {
    match v {
        Value::Int(n) => Ok(*n),
        _ => Err(Error::Decode(format!("{ctx}: expected Int, got {v:?}"))),
    }
}

fn expect_long(v: &Value, ctx: &'static str) -> Result<i64, Error> {
    match v {
        Value::Long(n) => Ok(*n),
        Value::Int(n) => Ok(*n as i64), // be lenient
        _ => Err(Error::Decode(format!("{ctx}: expected Long, got {v:?}"))),
    }
}

fn expect_string(v: &Value, ctx: &'static str) -> Result<String, Error> {
    match v {
        Value::String(s) => Ok(s.clone()),
        // Upstream callers (e.g. push_string) emit a None POD in place of
        // a null C string; printf("(null)") in the C tool turns it back
        // into a literal "(null)" — match that for byte-parity.
        Value::None => Ok("(null)".to_string()),
        _ => Err(Error::Decode(format!("{ctx}: expected String, got {v:?}"))),
    }
}

/// Convenience: is this proxy id the Core?
pub fn is_core(id: u32) -> bool {
    id == ID_CORE
}

/// Convenience: convert an interface short-name (e.g. "Node") back to the
/// full type string. Returns the input unchanged if it already looks fully
/// qualified.
pub fn full_interface_name(s: &str) -> String {
    if s.starts_with("PipeWire:Interface:") {
        s.to_string()
    } else {
        format!("{}{s}", interfaces::TYPE_CORE.trim_end_matches("Core"))
    }
}

/// Permission bitmasks (matching `src/pipewire/permission.h`).
/// Note: these are octal-encoded in the upstream header, not 1,2,4,8,16.
pub const PERM_R: u32 = 0o400;
pub const PERM_W: u32 = 0o200;
pub const PERM_X: u32 = 0o100;
pub const PERM_M: u32 = 0o010;
pub const PERM_L: u32 = 0o020;

/// Render a permission bitmask in the same `rwxml` style as `pw-cli`.
pub fn fmt_permissions(p: u32) -> String {
    let mut s = String::with_capacity(5);
    s.push(if p & PERM_R != 0 { 'r' } else { '-' });
    s.push(if p & PERM_W != 0 { 'w' } else { '-' });
    s.push(if p & PERM_X != 0 { 'x' } else { '-' });
    s.push(if p & PERM_M != 0 { 'm' } else { '-' });
    s.push(if p & PERM_L != 0 { 'l' } else { '-' });
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_decode_dict_roundtrip() {
        let d = build_dict(&[("a", "1"), ("b", "two")]);
        let items = decode_dict_struct(&d).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].key, "a");
        assert_eq!(items[0].value, "1");
        assert_eq!(items[1].key, "b");
        assert_eq!(items[1].value, "two");
    }

    #[test]
    fn empty_dict_roundtrip() {
        let d = build_dict(&[]);
        let items = decode_dict_struct(&d).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn core_info_decode() {
        let dict = build_dict(&[("core.name", "pipewire-0")]);
        let args = vec![
            Value::Int(0),
            Value::Int(0xdead_beefu32 as i32),
            Value::String("user".into()),
            Value::String("host".into()),
            Value::String("1.6.3".into()),
            Value::String("pipewire-0".into()),
            Value::Long(1),
            dict,
        ];
        let info = decode_core_info(&args).unwrap();
        assert_eq!(info.user_name, "user");
        assert_eq!(info.props.len(), 1);
        assert_eq!(info.props[0].key, "core.name");
    }

    #[test]
    fn registry_global_decode() {
        let dict = build_dict(&[("media.class", "Audio/Sink")]);
        let args = vec![
            Value::Int(42),
            Value::Int(7),
            Value::String("PipeWire:Interface:Node".into()),
            Value::Int(3),
            dict,
        ];
        let g = decode_registry_global(&args).unwrap();
        assert_eq!(g.id, 42);
        assert_eq!(g.permissions, 7);
        assert_eq!(g.interface, "PipeWire:Interface:Node");
        assert_eq!(g.version, 3);
        assert_eq!(g.props[0].value, "Audio/Sink");
    }

    #[test]
    fn full_name_helper() {
        assert_eq!(
            full_interface_name("Node"),
            "PipeWire:Interface:Node"
        );
        assert_eq!(
            full_interface_name("PipeWire:Interface:Node"),
            "PipeWire:Interface:Node"
        );
    }

    #[test]
    fn module_info_decode() {
        let dict = build_dict(&[("module.name", "libpipewire-module-foo")]);
        let args = vec![
            Value::Int(7),
            Value::String("foo".into()),
            Value::String("/path/foo.so".into()),
            Value::None, // null args
            Value::Long(1),
            dict,
        ];
        let info = decode_module_info(&args).unwrap();
        assert_eq!(info.id, 7);
        assert_eq!(info.name, "foo");
        assert_eq!(info.filename, "/path/foo.so");
        // None POD becomes "(null)" — matches printf("%s", NULL).
        assert_eq!(info.args, "(null)");
        assert_eq!(info.props.len(), 1);
    }

    #[test]
    fn factory_info_decode() {
        let dict = build_dict(&[("factory.name", "client-node")]);
        let args = vec![
            Value::Int(11),
            Value::String("client-node".into()),
            Value::String("PipeWire:Interface:ClientNode".into()),
            Value::Int(6),
            Value::Long(1),
            dict,
        ];
        let info = decode_factory_info(&args).unwrap();
        assert_eq!(info.id, 11);
        assert_eq!(info.object_type, "PipeWire:Interface:ClientNode");
        assert_eq!(info.version, 6);
    }

    #[test]
    fn node_info_decode() {
        let dict = build_dict(&[("node.name", "test")]);
        let params = Value::Struct(vec![
            Value::Int(2),
            Value::Id(1),
            Value::Int(0o002), // PARAM_INFO_READ
            Value::Id(2),
            Value::Int(0o006), // READ | WRITE
        ]);
        let args = vec![
            Value::Int(8),
            Value::Int(1),       // max_input_ports
            Value::Int(0),       // max_output_ports
            Value::Long(0x1f),   // change_mask all
            Value::Int(1),       // n_input_ports
            Value::Int(0),       // n_output_ports
            Value::Id(1),        // state = suspended
            Value::None,         // error null
            dict,
            params,
        ];
        let info = decode_node_info(&args).unwrap();
        assert_eq!(info.id, 8);
        assert_eq!(info.state, 1);
        assert_eq!(info.error, "(null)");
        assert_eq!(info.params.len(), 2);
        assert_eq!(info.params[0].id, 1);
        assert_eq!(info.params[0].flags, 0o002);
    }

    #[test]
    fn link_info_decode_no_format() {
        let dict = build_dict(&[]);
        let args = vec![
            Value::Int(20),
            Value::Int(11),  // out_node
            Value::Int(12),  // out_port
            Value::Int(13),  // in_node
            Value::Int(14),  // in_port
            Value::Long(7),
            Value::Int(0),   // state
            Value::String("".into()),
            Value::None,     // no format
            dict,
        ];
        let info = decode_link_info(&args).unwrap();
        assert_eq!(info.id, 20);
        assert_eq!(info.output_node_id, 11);
        assert_eq!(info.input_port_id, 14);
        assert!(info.format.is_none());
    }

    #[test]
    fn permissions_octal_format() {
        // PW_PERM_RWXM = 0o400 | 0o200 | 0o100 | 0o010 = 0o710
        assert_eq!(fmt_permissions(0o710), "rwxm-");
        assert_eq!(fmt_permissions(0o010), "---m-");
        assert_eq!(fmt_permissions(0o500), "r-x--");
        assert_eq!(fmt_permissions(0), "-----");
        assert_eq!(fmt_permissions(0o430), "r--ml");
    }
}
