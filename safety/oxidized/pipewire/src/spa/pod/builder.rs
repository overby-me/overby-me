// POD encoder: writes a Value tree as the canonical wire format.

use super::types::*;

/// Encode a single POD value to bytes.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    Builder::new(&mut out).write(value);
    out
}

pub struct Builder<'a> {
    out: &'a mut Vec<u8>,
}

impl<'a> Builder<'a> {
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        Self { out }
    }

    /// Append a complete POD (8-byte header + body + padding).
    pub fn write(&mut self, value: &Value) {
        let (ty, body) = encode_body(value);
        self.write_header(ty, body.len() as u32);
        self.out.extend_from_slice(&body);
        self.pad8();
    }

    fn write_header(&mut self, ty: u32, size: u32) {
        self.out.extend_from_slice(&size.to_le_bytes());
        self.out.extend_from_slice(&ty.to_le_bytes());
    }

    fn pad8(&mut self) {
        let n = align8(self.out.len()) - self.out.len();
        for _ in 0..n {
            self.out.push(0);
        }
    }
}

/// Returns (type, body bytes) for a single value. The body length goes
/// into the POD header. For container types the body itself is built by
/// recursive calls and includes any inner padding.
fn encode_body(value: &Value) -> (u32, Vec<u8>) {
    match value {
        Value::None => (SPA_TYPE_None, Vec::new()),
        Value::Bool(b) => (SPA_TYPE_Bool, encode_i32(if *b { 1 } else { 0 })),
        Value::Id(v) => (SPA_TYPE_Id, encode_u32(*v)),
        Value::Int(v) => (SPA_TYPE_Int, encode_i32(*v)),
        Value::Long(v) => (SPA_TYPE_Long, v.to_le_bytes().to_vec()),
        Value::Float(v) => {
            let mut b = Vec::with_capacity(4);
            b.extend_from_slice(&v.to_le_bytes());
            (SPA_TYPE_Float, b)
        }
        Value::Double(v) => (SPA_TYPE_Double, v.to_le_bytes().to_vec()),
        Value::String(s) => {
            // C strings are null-terminated; size includes the trailing NUL.
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            (SPA_TYPE_String, b)
        }
        Value::Bytes(b) => (SPA_TYPE_Bytes, b.clone()),
        Value::Rectangle(r) => {
            let mut b = Vec::with_capacity(8);
            b.extend_from_slice(&r.width.to_le_bytes());
            b.extend_from_slice(&r.height.to_le_bytes());
            (SPA_TYPE_Rectangle, b)
        }
        Value::Fraction(f) => {
            let mut b = Vec::with_capacity(8);
            b.extend_from_slice(&f.num.to_le_bytes());
            b.extend_from_slice(&f.denom.to_le_bytes());
            (SPA_TYPE_Fraction, b)
        }
        Value::Bitmap(b) => (SPA_TYPE_Bitmap, b.clone()),
        Value::Array {
            child_type,
            child_size,
            elements,
        } => {
            let mut body = Vec::new();
            // Array body header is itself a `struct spa_pod` (size, type)
            // describing each child.
            body.extend_from_slice(&child_size.to_le_bytes());
            body.extend_from_slice(&child_type.to_le_bytes());
            for el in elements {
                let (et, eb) = encode_body(el);
                debug_assert_eq!(et, *child_type, "array element type mismatch");
                body.extend_from_slice(&eb);
                // Elements in arrays are NOT individually padded — the
                // body is dense and the trailing pad goes on the array.
            }
            (SPA_TYPE_Array, body)
        }
        Value::Struct(items) => {
            let mut body = Vec::new();
            for it in items {
                let (et, eb) = encode_body(it);
                body.extend_from_slice(&(eb.len() as u32).to_le_bytes());
                body.extend_from_slice(&et.to_le_bytes());
                body.extend_from_slice(&eb);
                pad_to_8(&mut body);
            }
            (SPA_TYPE_Struct, body)
        }
        Value::Object {
            object_type,
            id,
            properties,
        } => {
            let mut body = Vec::new();
            body.extend_from_slice(&object_type.to_le_bytes());
            body.extend_from_slice(&id.to_le_bytes());
            for p in properties {
                let (vt, vb) = encode_body(&p.value);
                body.extend_from_slice(&p.key.to_le_bytes());
                body.extend_from_slice(&p.flags.to_le_bytes());
                // The property's value is a complete POD (with its own header).
                body.extend_from_slice(&(vb.len() as u32).to_le_bytes());
                body.extend_from_slice(&vt.to_le_bytes());
                body.extend_from_slice(&vb);
                pad_to_8(&mut body);
            }
            (SPA_TYPE_Object, body)
        }
        Value::Sequence { unit, controls } => {
            let mut body = Vec::new();
            body.extend_from_slice(&unit.to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes()); // pad
            for c in controls {
                let (vt, vb) = encode_body(&c.value);
                body.extend_from_slice(&c.offset.to_le_bytes());
                body.extend_from_slice(&c.ctrl_type.to_le_bytes());
                body.extend_from_slice(&(vb.len() as u32).to_le_bytes());
                body.extend_from_slice(&vt.to_le_bytes());
                body.extend_from_slice(&vb);
                pad_to_8(&mut body);
            }
            (SPA_TYPE_Sequence, body)
        }
        Value::Pointer { ptype, ptr } => {
            let mut body = Vec::with_capacity(16);
            body.extend_from_slice(&ptype.to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
            body.extend_from_slice(&ptr.to_le_bytes());
            (SPA_TYPE_Pointer, body)
        }
        Value::Fd(v) => (SPA_TYPE_Fd, v.to_le_bytes().to_vec()),
        Value::Choice {
            choice_type,
            flags,
            child_type,
            child_size,
            elements,
        } => {
            let mut body = Vec::new();
            body.extend_from_slice(&choice_type.to_le_bytes());
            body.extend_from_slice(&flags.to_le_bytes());
            body.extend_from_slice(&child_size.to_le_bytes());
            body.extend_from_slice(&child_type.to_le_bytes());
            for el in elements {
                let (et, eb) = encode_body(el);
                debug_assert_eq!(et, *child_type);
                body.extend_from_slice(&eb);
            }
            (SPA_TYPE_Choice, body)
        }
    }
}

fn encode_i32(v: i32) -> Vec<u8> {
    let mut b = Vec::with_capacity(4);
    b.extend_from_slice(&v.to_le_bytes());
    b
}

fn encode_u32(v: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(4);
    b.extend_from_slice(&v.to_le_bytes());
    b
}

fn pad_to_8(body: &mut Vec<u8>) {
    let n = align8(body.len()) - body.len();
    for _ in 0..n {
        body.push(0);
    }
}
