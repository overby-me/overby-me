// Round-trip POD encoding/decoding tests covering every value kind.
// Mirrors the assertions in upstream test/test-spa-pod.c.

use super::types::*;
use super::*;

fn roundtrip(v: Value) {
    let bytes = builder::encode(&v);
    // The encoded length must be 8-byte aligned.
    assert_eq!(bytes.len() % 8, 0, "POD length not 8-byte aligned");
    let (decoded, used) = parser::read_pod(&bytes).unwrap();
    assert_eq!(used, bytes.len());
    assert_eq!(decoded, v);
}

#[test]
fn pod_none_is_8_bytes() {
    let bytes = builder::encode(&Value::None);
    assert_eq!(bytes.len(), 8, "None POD: just header, body=0");
    let size = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let ty = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(size, 0);
    assert_eq!(ty, SPA_TYPE_None);
}

#[test]
fn pod_int_is_16_bytes() {
    // C layout: { struct spa_pod (8 bytes), i32 value (4), i32 _padding (4) }
    // Our encoder writes 8 byte header + 4-byte body padded to 16.
    let bytes = builder::encode(&Value::Int(42));
    assert_eq!(bytes.len(), 16);
    let size = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let ty = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(size, 4);
    assert_eq!(ty, SPA_TYPE_Int);
    assert_eq!(i32::from_le_bytes(bytes[8..12].try_into().unwrap()), 42);
}

#[test]
fn pod_long_is_16_bytes() {
    let bytes = builder::encode(&Value::Long(0xdeadbeef));
    assert_eq!(bytes.len(), 16);
}

#[test]
fn primitives_roundtrip() {
    roundtrip(Value::None);
    roundtrip(Value::Bool(true));
    roundtrip(Value::Bool(false));
    roundtrip(Value::Id(0x1234));
    roundtrip(Value::Int(-7));
    roundtrip(Value::Long(0x1122_3344_5566_7788));
    roundtrip(Value::Float(1.5));
    roundtrip(Value::Double(std::f64::consts::PI));
    roundtrip(Value::String("hello".into()));
    roundtrip(Value::String("".into()));
    roundtrip(Value::Bytes(vec![1, 2, 3, 4, 5]));
    roundtrip(Value::Rectangle(Rectangle {
        width: 1920,
        height: 1080,
    }));
    roundtrip(Value::Fraction(Fraction {
        num: 30,
        denom: 1001,
    }));
    roundtrip(Value::Bitmap(vec![0xff, 0x00, 0xaa]));
    roundtrip(Value::Fd(42));
    roundtrip(Value::Pointer {
        ptype: 0x10001,
        ptr: 0,
    });
}

#[test]
fn array_of_ints() {
    let v = Value::Array {
        child_type: SPA_TYPE_Int,
        child_size: 4,
        elements: vec![Value::Int(1), Value::Int(2), Value::Int(3), Value::Int(4)],
    };
    roundtrip(v);
}

#[test]
fn struct_of_mixed() {
    let v = Value::Struct(vec![
        Value::Int(1),
        Value::String("foo".into()),
        Value::Bool(true),
        Value::Long(99),
    ]);
    roundtrip(v);
}

#[test]
fn nested_struct() {
    let v = Value::Struct(vec![
        Value::Int(1),
        Value::Struct(vec![Value::Int(2), Value::Int(3)]),
        Value::Int(4),
    ]);
    roundtrip(v);
}

#[test]
fn object_with_props() {
    let v = Value::Object {
        object_type: 0x40001,
        id: 5,
        properties: vec![
            Property {
                key: 1,
                flags: 0,
                value: Value::Id(7),
            },
            Property {
                key: 2,
                flags: 0,
                value: Value::String("audio.raw".into()),
            },
            Property {
                key: 3,
                flags: 0,
                value: Value::Rectangle(Rectangle {
                    width: 1920,
                    height: 1080,
                }),
            },
        ],
    };
    roundtrip(v);
}

#[test]
fn choice_enum() {
    let v = Value::Choice {
        choice_type: SPA_CHOICE_Enum,
        flags: 0,
        child_type: SPA_TYPE_Int,
        child_size: 4,
        elements: vec![Value::Int(1), Value::Int(2), Value::Int(4), Value::Int(8)],
    };
    roundtrip(v);
}

#[test]
fn choice_range() {
    let v = Value::Choice {
        choice_type: SPA_CHOICE_Range,
        flags: 0,
        child_type: SPA_TYPE_Long,
        child_size: 8,
        elements: vec![Value::Long(48000), Value::Long(8000), Value::Long(192000)],
    };
    roundtrip(v);
}

#[test]
fn sequence_of_controls() {
    let v = Value::Sequence {
        unit: 48000,
        controls: vec![
            Control {
                offset: 0,
                ctrl_type: 0,
                value: Value::Float(0.0),
            },
            Control {
                offset: 1024,
                ctrl_type: 1,
                value: Value::Float(0.5),
            },
        ],
    };
    roundtrip(v);
}

#[test]
fn deeply_nested() {
    let v = Value::Struct(vec![
        Value::Object {
            object_type: 0x40003, // Format
            id: 0,
            properties: vec![Property {
                key: 1,
                flags: 0,
                value: Value::Choice {
                    choice_type: SPA_CHOICE_Enum,
                    flags: 0,
                    child_type: SPA_TYPE_Int,
                    child_size: 4,
                    elements: vec![Value::Int(48000), Value::Int(44100)],
                },
            }],
        },
        Value::Array {
            child_type: SPA_TYPE_Float,
            child_size: 4,
            elements: vec![Value::Float(0.1), Value::Float(0.2), Value::Float(0.3)],
        },
    ]);
    roundtrip(v);
}

#[test]
fn string_padding_to_8() {
    // "hello" + NUL = 6 bytes → POD body=6, total=8+6=14, padded to 16.
    let bytes = builder::encode(&Value::String("hello".into()));
    assert_eq!(bytes.len(), 16);
    // Trailing bytes after the 6-byte payload should be zero padding.
    assert_eq!(&bytes[8..14], b"hello\0");
    assert_eq!(&bytes[14..16], &[0, 0]);
}

#[test]
fn empty_string_size_includes_nul() {
    let bytes = builder::encode(&Value::String("".into()));
    let size = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    assert_eq!(size, 1, "empty string body = single NUL byte");
    assert_eq!(bytes.len(), 16, "padded total length");
}
