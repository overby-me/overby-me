// Native protocol framing.
//
// Wire format from `src/modules/module-protocol-native/connection.c`:
//
// ```text
// struct {
//   u32 id;        // proxy/object id (0 = Core proxy)
//   u32 op_size;   // (opcode << 24) | (size & 0x00ffffff)
//   u32 seq;       // sequence number
//   u32 n_fds;     // number of fds in ancillary data
//   u8  body[size]; // a Struct POD carrying the call arguments
// };
// ```
//
// Byte order on the wire is host order — pipewire only ships little-endian
// architectures via its socket API. Header is 16 bytes (HDR_SIZE).

use crate::spa::pod::builder;
use crate::spa::pod::types::Value;

pub const HDR_SIZE: usize = 16;

/// A complete encoded protocol message ready to send (excluding fds, which
/// travel as ancillary data via SCM_RIGHTS).
#[derive(Debug, Clone)]
pub struct Message {
    pub bytes: Vec<u8>,
    #[allow(dead_code)]
    pub n_fds: u32,
}

/// Encode a single call: header + a single Struct POD wrapping `args`.
pub fn encode_call(id: u32, opcode: u8, seq: u32, args: &[Value]) -> Message {
    let body = builder::encode(&Value::Struct(args.to_vec()));
    let size = body.len();
    assert!(
        size <= 0x00ff_ffff,
        "POD body too large for 24-bit size field"
    );

    let mut msg = Vec::with_capacity(HDR_SIZE + size);
    msg.extend_from_slice(&id.to_le_bytes());
    msg.extend_from_slice(&((u32::from(opcode) << 24) | size as u32).to_le_bytes());
    msg.extend_from_slice(&seq.to_le_bytes());
    msg.extend_from_slice(&0u32.to_le_bytes()); // n_fds
    msg.extend_from_slice(&body);

    Message {
        bytes: msg,
        n_fds: 0,
    }
}

/// Decoded message: header fields + the inner Struct POD args.
#[derive(Debug, Clone)]
pub struct Decoded {
    pub id: u32,
    pub opcode: u8,
    pub seq: u32,
    pub n_fds: u32,
    pub args: Vec<Value>,
}

#[derive(Debug, Clone)]
pub struct DecodeError(pub String);
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for DecodeError {}

/// Decode a protocol message. Returns the parsed message plus the number
/// of bytes consumed (header + body).
pub fn decode(bytes: &[u8]) -> Result<(Decoded, usize), DecodeError> {
    if bytes.len() < HDR_SIZE {
        return Err(DecodeError("message header truncated".into()));
    }
    let id = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let op_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let opcode = (op_size >> 24) as u8;
    let size = (op_size & 0x00ff_ffff) as usize;
    let seq = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let n_fds = u32::from_le_bytes(bytes[12..16].try_into().unwrap());

    let total = HDR_SIZE + size;
    if bytes.len() < total {
        return Err(DecodeError("message body truncated".into()));
    }

    let body = &bytes[HDR_SIZE..total];
    let (root, _) = crate::spa::pod::parser::read_pod(body)
        .map_err(|e| DecodeError(format!("body POD parse: {e}")))?;
    let args = match root {
        Value::Struct(v) => v,
        other => {
            return Err(DecodeError(format!(
                "expected Struct POD body, got {other:?}"
            )));
        }
    };

    Ok((
        Decoded {
            id,
            opcode,
            seq,
            n_fds,
            args,
        },
        total,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spa::pod::types::Value;

    #[test]
    fn hello_message_layout() {
        // Core.Hello(version=3): id=0, opcode=1, seq=0, body = Struct{Int(3)}
        let m = encode_call(0, 1, 0, &[Value::Int(3)]);

        // Body: Struct(Int(3))
        // - Struct POD: size=16 (Int POD's full padded length), type=14
        // - Int POD: size=4, type=4, value=3, pad=4 → 16 bytes
        // Total body: 24 bytes
        // Header: 16 bytes
        // Total message: 40 bytes
        assert_eq!(m.bytes.len(), 40);

        // id
        assert_eq!(&m.bytes[0..4], &[0, 0, 0, 0]);
        // op_size = (1 << 24) | 24 = 0x01000018, LE
        assert_eq!(&m.bytes[4..8], &[0x18, 0x00, 0x00, 0x01]);
        // seq = 0
        assert_eq!(&m.bytes[8..12], &[0, 0, 0, 0]);
        // n_fds = 0
        assert_eq!(&m.bytes[12..16], &[0, 0, 0, 0]);

        // Struct POD header
        assert_eq!(&m.bytes[16..20], &16u32.to_le_bytes());
        assert_eq!(&m.bytes[20..24], &14u32.to_le_bytes()); // SPA_TYPE_Struct

        // Int POD header + value
        assert_eq!(&m.bytes[24..28], &4u32.to_le_bytes());
        assert_eq!(&m.bytes[28..32], &4u32.to_le_bytes()); // SPA_TYPE_Int
        assert_eq!(&m.bytes[32..36], &3u32.to_le_bytes());
        // padding
        assert_eq!(&m.bytes[36..40], &[0, 0, 0, 0]);
    }

    #[test]
    fn roundtrip_call() {
        let args = vec![
            Value::Int(3),
            Value::String("PipeWire:Interface:Node".into()),
            Value::Long(0xbeef),
        ];
        let m = encode_call(7, 4, 99, &args);
        let (d, used) = decode(&m.bytes).unwrap();
        assert_eq!(used, m.bytes.len());
        assert_eq!(d.id, 7);
        assert_eq!(d.opcode, 4);
        assert_eq!(d.seq, 99);
        assert_eq!(d.n_fds, 0);
        assert_eq!(d.args, args);
    }
}
