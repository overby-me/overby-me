// POD decoder: walks the wire format and produces a Value tree.

#![allow(non_upper_case_globals)]

use super::types::*;

#[derive(Debug, Clone)]
pub struct ParseError(pub String);
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ParseError {}

pub type Result<T> = std::result::Result<T, ParseError>;

pub struct Parser<'a> {
    bytes: &'a [u8],
}

impl<'a> Parser<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Parse a single complete POD (with header + body) starting at offset 0.
    /// Returns the parsed value and the number of bytes consumed (including
    /// trailing padding).
    pub fn read_pod(&self) -> Result<(Value, usize)> {
        read_pod(self.bytes)
    }
}

/// Read a complete POD with its 8-byte header at `bytes[0..]`.
pub fn read_pod(bytes: &[u8]) -> Result<(Value, usize)> {
    if bytes.len() < 8 {
        return Err(ParseError("POD header truncated".into()));
    }
    let size = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let ty = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let total = 8 + size;
    if bytes.len() < total {
        return Err(ParseError("POD body truncated".into()));
    }
    let body = &bytes[8..total];
    let value = read_body(ty, body)?;
    let padded = align8(total);
    Ok((value, padded.min(bytes.len())))
}

/// Read a POD body of known type and exact length.
pub fn read_body(ty: u32, body: &[u8]) -> Result<Value> {
    match ty {
        SPA_TYPE_None => Ok(Value::None),
        SPA_TYPE_Bool => {
            need(body, 4, "Bool")?;
            let v = i32::from_le_bytes(body[0..4].try_into().unwrap());
            Ok(Value::Bool(v != 0))
        }
        SPA_TYPE_Id => {
            need(body, 4, "Id")?;
            Ok(Value::Id(u32::from_le_bytes(
                body[0..4].try_into().unwrap(),
            )))
        }
        SPA_TYPE_Int => {
            need(body, 4, "Int")?;
            Ok(Value::Int(i32::from_le_bytes(
                body[0..4].try_into().unwrap(),
            )))
        }
        SPA_TYPE_Long => {
            need(body, 8, "Long")?;
            Ok(Value::Long(i64::from_le_bytes(
                body[0..8].try_into().unwrap(),
            )))
        }
        SPA_TYPE_Float => {
            need(body, 4, "Float")?;
            Ok(Value::Float(f32::from_le_bytes(
                body[0..4].try_into().unwrap(),
            )))
        }
        SPA_TYPE_Double => {
            need(body, 8, "Double")?;
            Ok(Value::Double(f64::from_le_bytes(
                body[0..8].try_into().unwrap(),
            )))
        }
        SPA_TYPE_String => {
            // Strings include a trailing NUL counted in size.
            let end = body.iter().position(|&b| b == 0).unwrap_or(body.len());
            let s = std::str::from_utf8(&body[..end])
                .map_err(|_| ParseError("invalid UTF-8 in String POD".into()))?
                .to_string();
            Ok(Value::String(s))
        }
        SPA_TYPE_Bytes => Ok(Value::Bytes(body.to_vec())),
        SPA_TYPE_Rectangle => {
            need(body, 8, "Rectangle")?;
            Ok(Value::Rectangle(Rectangle {
                width: u32::from_le_bytes(body[0..4].try_into().unwrap()),
                height: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            }))
        }
        SPA_TYPE_Fraction => {
            need(body, 8, "Fraction")?;
            Ok(Value::Fraction(Fraction {
                num: u32::from_le_bytes(body[0..4].try_into().unwrap()),
                denom: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            }))
        }
        SPA_TYPE_Bitmap => Ok(Value::Bitmap(body.to_vec())),
        SPA_TYPE_Array => {
            need(body, 8, "Array header")?;
            let child_size = u32::from_le_bytes(body[0..4].try_into().unwrap());
            let child_type = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let mut elements = Vec::new();
            let mut p = 8;
            while p < body.len() {
                let end = p + child_size as usize;
                if end > body.len() {
                    return Err(ParseError("Array element truncated".into()));
                }
                elements.push(read_body(child_type, &body[p..end])?);
                p = end;
            }
            Ok(Value::Array {
                child_type,
                child_size,
                elements,
            })
        }
        SPA_TYPE_Struct => {
            let mut items = Vec::new();
            let mut p = 0;
            while p < body.len() {
                let (v, used) = read_pod(&body[p..])?;
                items.push(v);
                p += used;
            }
            Ok(Value::Struct(items))
        }
        SPA_TYPE_Object => {
            need(body, 8, "Object header")?;
            let object_type = u32::from_le_bytes(body[0..4].try_into().unwrap());
            let id = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let mut properties = Vec::new();
            let mut p = 8;
            while p < body.len() {
                if p + 16 > body.len() {
                    return Err(ParseError("Object property header truncated".into()));
                }
                let key = u32::from_le_bytes(body[p..p + 4].try_into().unwrap());
                let flags = u32::from_le_bytes(body[p + 4..p + 8].try_into().unwrap());
                let v_size = u32::from_le_bytes(body[p + 8..p + 12].try_into().unwrap()) as usize;
                let v_type = u32::from_le_bytes(body[p + 12..p + 16].try_into().unwrap());
                let v_start = p + 16;
                let v_end = v_start + v_size;
                if v_end > body.len() {
                    return Err(ParseError("Object property value truncated".into()));
                }
                let value = read_body(v_type, &body[v_start..v_end])?;
                properties.push(Property { key, flags, value });
                // The C layout pads each property's value POD to 8 bytes.
                let consumed = 16 + v_size;
                p += align8(consumed);
            }
            Ok(Value::Object {
                object_type,
                id,
                properties,
            })
        }
        SPA_TYPE_Sequence => {
            need(body, 8, "Sequence header")?;
            let unit = u32::from_le_bytes(body[0..4].try_into().unwrap());
            let mut p = 8;
            let mut controls = Vec::new();
            while p < body.len() {
                if p + 16 > body.len() {
                    return Err(ParseError("Sequence control header truncated".into()));
                }
                let offset = u32::from_le_bytes(body[p..p + 4].try_into().unwrap());
                let ctrl_type = u32::from_le_bytes(body[p + 4..p + 8].try_into().unwrap());
                let v_size = u32::from_le_bytes(body[p + 8..p + 12].try_into().unwrap()) as usize;
                let v_type = u32::from_le_bytes(body[p + 12..p + 16].try_into().unwrap());
                let v_start = p + 16;
                let v_end = v_start + v_size;
                if v_end > body.len() {
                    return Err(ParseError("Sequence control value truncated".into()));
                }
                let value = read_body(v_type, &body[v_start..v_end])?;
                controls.push(Control {
                    offset,
                    ctrl_type,
                    value,
                });
                let consumed = 16 + v_size;
                p += align8(consumed);
            }
            Ok(Value::Sequence { unit, controls })
        }
        SPA_TYPE_Pointer => {
            need(body, 16, "Pointer")?;
            let ptype = u32::from_le_bytes(body[0..4].try_into().unwrap());
            let ptr = u64::from_le_bytes(body[8..16].try_into().unwrap());
            Ok(Value::Pointer { ptype, ptr })
        }
        SPA_TYPE_Fd => {
            need(body, 8, "Fd")?;
            Ok(Value::Fd(i64::from_le_bytes(
                body[0..8].try_into().unwrap(),
            )))
        }
        SPA_TYPE_Choice => {
            need(body, 16, "Choice header")?;
            let choice_type = u32::from_le_bytes(body[0..4].try_into().unwrap());
            let flags = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let child_size = u32::from_le_bytes(body[8..12].try_into().unwrap());
            let child_type = u32::from_le_bytes(body[12..16].try_into().unwrap());
            let mut elements = Vec::new();
            let mut p = 16;
            while p < body.len() {
                let end = p + child_size as usize;
                if end > body.len() {
                    return Err(ParseError("Choice element truncated".into()));
                }
                elements.push(read_body(child_type, &body[p..end])?);
                p = end;
            }
            Ok(Value::Choice {
                choice_type,
                flags,
                child_type,
                child_size,
                elements,
            })
        }
        _ => Err(ParseError(format!("unknown POD type {ty}"))),
    }
}

fn need(body: &[u8], n: usize, label: &str) -> Result<()> {
    if body.len() < n {
        Err(ParseError(format!(
            "{label} body too short ({} < {n})",
            body.len()
        )))
    } else {
        Ok(())
    }
}
