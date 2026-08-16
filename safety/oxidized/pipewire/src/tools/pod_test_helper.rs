// Helper used by the POD comparison harness: encodes a Rust Value tree
// from CLI arguments and writes the binary POD to stdout. The companion
// C test harness encodes the same value via libspa and we diff the output.
//
// Invoked as: rust-pipewire pod-encode <case>
//
// `<case>` selects a hard-coded sample value so the C and Rust sides agree
// on what they're encoding.

use crate::spa::pod::{builder, types::*};
use std::io::Write;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pod-encode");
    let case = match args.get(1) {
        Some(c) => c.as_str(),
        None => {
            eprintln!("{argv0} <case-name>");
            return 2;
        }
    };

    let value = match sample(case) {
        Some(v) => v,
        None => {
            eprintln!("{argv0}: unknown case {case:?}");
            return 2;
        }
    };

    let bytes = builder::encode(&value);
    if std::io::stdout().write_all(&bytes).is_err() {
        return 1;
    }
    0
}

/// Sample values keyed by name; the C and Rust harnesses must build the
/// same value for the same key.
pub fn sample(name: &str) -> Option<Value> {
    Some(match name {
        "none" => Value::None,
        "bool-true" => Value::Bool(true),
        "bool-false" => Value::Bool(false),
        "id-7" => Value::Id(7),
        "int-42" => Value::Int(42),
        "int-neg" => Value::Int(-7),
        "long-pi" => Value::Long(0x1122_3344_5566_7788),
        "float-1_5" => Value::Float(1.5),
        "double-pi" => Value::Double(std::f64::consts::PI),
        "string-hello" => Value::String("hello".into()),
        "string-empty" => Value::String("".into()),
        "rect-fhd" => Value::Rectangle(Rectangle {
            width: 1920,
            height: 1080,
        }),
        "frac-30000-1001" => Value::Fraction(Fraction {
            num: 30000,
            denom: 1001,
        }),
        "fd-3" => Value::Fd(3),
        "array-int-1234" => Value::Array {
            child_type: SPA_TYPE_Int,
            child_size: 4,
            elements: vec![Value::Int(1), Value::Int(2), Value::Int(3), Value::Int(4)],
        },
        "struct-mixed" => Value::Struct(vec![
            Value::Int(1),
            Value::String("foo".into()),
            Value::Bool(true),
            Value::Long(99),
        ]),
        "object-format" => Value::Object {
            object_type: 0x40003, // OBJECT_Format
            id: 0,
            properties: vec![
                Property {
                    key: 1,
                    flags: 0,
                    value: Value::Id(0x10001),
                },
                Property {
                    key: 2,
                    flags: 0,
                    value: Value::String("audio.raw".into()),
                },
            ],
        },
        "choice-enum-rates" => Value::Choice {
            choice_type: SPA_CHOICE_Enum,
            flags: 0,
            child_type: SPA_TYPE_Int,
            child_size: 4,
            elements: vec![Value::Int(48000), Value::Int(44100), Value::Int(96000)],
        },
        _ => return None,
    })
}
