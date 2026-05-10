// pw-dump: dump a running PipeWire registry as JSON.
//
// Phase-7 minimal mode: connects, walks the registry, emits one JSON object
// per global with id/type/version/permissions/props (registry-side props
// only — the C tool also binds each global and emits an "info" block;
// that's deferred to the per-interface dumpers in Phase 8).

use crate::pipewire_lib::client::{Client, RegistryGlobal};
use crate::pipewire_lib::interfaces;
use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("pw-dump");
    let args = expand_short_clusters(raw_args, &['h', 'V', 'N', 'R', 's', 'm']);

    let mut indent: usize = 2;
    // Note: -- terminator is handled in the loop below.
    let mut remote: Option<String> = None;
    let mut filter: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => {
                // End of options; rest are positional id/interface filters.
                if let Some(v) = args.get(i + 1) {
                    filter = Some(v.clone());
                }
                break;
            }
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "--version" | "-V" => {
                print_version(argv0);
                return 0;
            }
            s if s.starts_with("--help=") => {
                eprintln!("{argv0}: option '--help' doesn't allow an argument");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with("--no-colors=")
                || s.starts_with("--raw=")
                || s.starts_with("--spa=")
                || s.starts_with("--monitor=") =>
            {
                let name = s.split_once('=').map(|(n, _)| n).unwrap_or(s);
                eprintln!("{argv0}: option '{name}' doesn't allow an argument");
                print_help(argv0);
                return 0;
            }
            "-i" | "--indent" => {
                if i + 1 >= args.len() {
                    eprintln!("{argv0}: option requires an argument -- 'i'");
                    print_help(argv0);
                    return 0;
                }
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<usize>().ok()) {
                    indent = v;
                }
                i += 2;
                continue;
            }
            opt @ ("-r" | "--remote") => {
                if i + 1 >= args.len() {
                    if opt == "--remote" {
                        eprintln!("{argv0}: option '--remote' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'r'");
                    }
                    print_help(argv0);
                    return 0;
                }
                remote = Some(args[i + 1].clone());
                i += 2;
                continue;
            }
            s if s.starts_with("--remote=") => {
                remote = Some(s["--remote=".len()..].to_string());
            }
            s if s.starts_with("-r") && s.len() > 2 => {
                remote = Some(s[2..].to_string());
            }
            "-N" | "--no-colors" | "-R" | "--raw" | "-s" | "--spa" | "-m" | "--monitor" => {
                // Accepted but ignored at this phase.
            }
            "-" => {
                // Lone `-` is a positional id/interface filter, not an
                // option. Use it as the filter (won't match anything).
                filter = Some("-".to_string());
            }
            "--color" | "-C" => {
                // Bare flag → defaults to "auto"; OK.
            }
            s if s.starts_with("--color=") => {
                let val = &s["--color=".len()..];
                if !matches!(val, "" | "never" | "always" | "auto") {
                    eprintln!("Unknown color: {val}");
                    print_help(argv0);
                    return 255;
                }
            }
            s if s.starts_with("-C") && s.len() > 2 => {
                let val = &s[2..];
                if !matches!(val, "never" | "always" | "auto") {
                    eprintln!("Unknown color: {val}");
                    print_help(argv0);
                    return 255;
                }
            }
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with('-') && !s.starts_with("--") => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with('-') => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            s => {
                // Positional: object id (numeric) or interface name.
                filter = Some(s.to_string());
            }
        }
        i += 1;
    }

    let globals = match collect_globals(remote.as_deref()) {
        Ok(g) => g,
        Err(e) => {
            // C pw-dump prints `can't connect: <strerror>` (no `Error:`
            // wrapper) and exits 255.
            if e.contains("connect:") || e.starts_with("connect:") {
                eprintln!(
                    "can't connect: {}",
                    crate::tools::common::connect_failure_msg()
                );
            } else {
                eprintln!("{argv0}: {e}");
            }
            return 255;
        }
    };

    let filter_id: Option<u32> = filter.as_deref().and_then(|s| s.parse().ok());
    let filter_iface: Option<String> = filter.as_ref().and_then(|s| {
        if s.parse::<u32>().is_ok() {
            None
        } else {
            Some(if s.contains(':') {
                s.clone()
            } else {
                format!("PipeWire:Interface:{s}")
            })
        }
    });

    let mut sorted: Vec<&RegistryGlobal> = globals
        .iter()
        .filter(|g| filter_id.is_none_or(|f| g.id == f))
        .filter(|g| filter_iface.as_deref().is_none_or(|f| g.interface == f))
        .collect();
    sorted.sort_by_key(|g| g.id);

    let mut out = String::new();
    write_array(&mut out, &sorted, indent);
    println!("{out}");
    0
}

fn print_help(argv0: &str) {
    println!("{argv0} [options] [<id>]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -r, --remote                          Remote daemon name");
    println!("  -m, --monitor                         monitor changes");
    println!("  -N, --no-colors                       disable color output");
    println!(
        "  -C, --color[=WHEN]                    whether to enable color support. WHEN is `never`, `always`, or `auto`"
    );
    println!("  -R, --raw                             force raw output");
    println!("  -i, --indent                          indentation amount (default 2)");
    println!("  -s, --spa                             SPA JSON output");
}

fn collect_globals(remote: Option<&str>) -> Result<Vec<RegistryGlobal>, String> {
    let mut client = match remote {
        Some(name) if name.starts_with('/') => Client::connect_path(std::path::Path::new(name)),
        Some(name) => {
            let runtime = std::env::var("PIPEWIRE_RUNTIME_DIR")
                .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
                .unwrap_or_else(|_| "/tmp".to_string());
            let path = std::path::PathBuf::from(runtime).join(name);
            Client::connect_path(&path)
        }
        None => Client::connect_default(),
    }
    .map_err(|e| format!("connect: {e}"))?;

    client
        .handshake("rust-pipewire-dump")
        .map_err(|e| format!("handshake: {e}"))?;
    let sync_seq = client
        .sync(interfaces::ID_CORE)
        .map_err(|e| format!("sync: {e}"))?;

    let mut globals = Vec::new();
    loop {
        let msg = match client.read_message() {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(e) => return Err(format!("read: {e}")),
        };
        if msg.opcode == interfaces::registry_event::GLOBAL
            && msg.id == 2
            && let Ok(g) = crate::pipewire_lib::client::decode_registry_global(&msg.args)
        {
            globals.push(g);
        }
        if msg.opcode == interfaces::registry_event::GLOBAL_REMOVE
            && msg.id == 2
            && let Ok(rid) = crate::pipewire_lib::client::decode_registry_global_remove(&msg.args)
        {
            globals.retain(|g: &RegistryGlobal| g.id != rid);
        }
        if msg.id == interfaces::ID_CORE
            && msg.opcode == interfaces::core_event::DONE
            && let Ok((_id, seq)) = crate::pipewire_lib::client::decode_core_done(&msg.args)
            && seq == sync_seq
        {
            break;
        }
    }
    Ok(globals)
}

fn write_array(out: &mut String, globals: &[&RegistryGlobal], indent: usize) {
    if globals.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push('[');
    for (i, g) in globals.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_newline_indent(out, indent, 1);
        write_global(out, g, indent, 1);
    }
    push_newline_indent(out, indent, 0);
    out.push(']');
}

fn write_global(out: &mut String, g: &RegistryGlobal, indent: usize, level: usize) {
    out.push('{');
    push_newline_indent(out, indent, level + 1);
    out.push_str(&format!("\"id\": {},", g.id));
    push_newline_indent(out, indent, level + 1);
    out.push_str(&format!("\"type\": \"{}\",", json_escape(&g.interface)));
    push_newline_indent(out, indent, level + 1);
    out.push_str(&format!("\"version\": {},", g.version));
    push_newline_indent(out, indent, level + 1);
    out.push_str("\"permissions\": ");
    write_permissions(out, g.permissions);
    out.push(',');
    push_newline_indent(out, indent, level + 1);
    out.push_str("\"props\": ");
    write_props(out, &g.props, indent, level + 1);
    push_newline_indent(out, indent, level);
    out.push('}');
}

fn write_permissions(out: &mut String, perms: u32) {
    use crate::pipewire_lib::client::{PERM_L, PERM_M, PERM_R, PERM_W, PERM_X};
    out.push('[');
    let mut first = true;
    for (mask, name) in [
        (PERM_R, "r"),
        (PERM_W, "w"),
        (PERM_X, "x"),
        (PERM_M, "m"),
        (PERM_L, "l"),
    ] {
        if perms & mask != 0 {
            if !first {
                out.push(',');
            }
            out.push_str(&format!(" \"{name}\""));
            first = false;
        }
    }
    if first {
        out.push(']');
    } else {
        out.push_str(" ]");
    }
}

fn write_props(
    out: &mut String,
    items: &[crate::pipewire_lib::client::DictItem],
    indent: usize,
    level: usize,
) {
    if items.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push('{');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_newline_indent(out, indent, level + 1);
        out.push_str(&format!(
            "\"{}\": {}",
            json_escape(&item.key),
            render_dict_value(&item.value)
        ));
    }
    push_newline_indent(out, indent, level);
    out.push('}');
}

/// Mirror the C tool's `put_value`: numeric/bool/null literals stay raw,
/// otherwise emit a quoted string. We only treat a string as a number when
/// reformatting it doesn't change the digits (so "007" stays a string —
/// JSON doesn't allow leading zeros).
fn render_dict_value(v: &str) -> String {
    if v == "true" || v == "false" {
        return v.to_string();
    }
    if let Ok(n) = v.parse::<i64>()
        && n.to_string() == v
    {
        return v.to_string();
    }
    if v.contains(['.', 'e', 'E'])
        && let Ok(f) = v.parse::<f64>()
        && f.is_finite()
    {
        return v.to_string();
    }
    format!("\"{}\"", json_escape(v))
}

fn push_newline_indent(out: &mut String, indent: usize, level: usize) {
    if indent == 0 {
        return;
    }
    out.push('\n');
    for _ in 0..indent * level {
        out.push(' ');
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipewire_lib::client::DictItem;

    fn g(id: u32, iface: &str, version: u32, props: &[(&str, &str)]) -> RegistryGlobal {
        RegistryGlobal {
            id,
            // r|w|x: 0o400 | 0o200 | 0o100 = 0o700.
            permissions: 0o700,
            interface: iface.into(),
            version,
            props: props
                .iter()
                .map(|(k, v)| DictItem {
                    key: (*k).into(),
                    value: (*v).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn empty_array() {
        let mut s = String::new();
        write_array(&mut s, &[], 2);
        assert_eq!(s, "[]");
    }

    #[test]
    fn one_global() {
        let g0 = g(
            0,
            "PipeWire:Interface:Core",
            4,
            &[("core.name", "pipewire-0")],
        );
        let mut s = String::new();
        write_array(&mut s, &[&g0], 2);
        let expected = "[\n  {\n    \"id\": 0,\n    \"type\": \"PipeWire:Interface:Core\",\n    \"version\": 4,\n    \"permissions\": [ \"r\", \"w\", \"x\" ],\n    \"props\": {\n      \"core.name\": \"pipewire-0\"\n    }\n  }\n]";
        assert_eq!(s, expected);
    }

    #[test]
    fn dict_values_typed() {
        assert_eq!(render_dict_value("42"), "42");
        assert_eq!(render_dict_value("true"), "true");
        assert_eq!(render_dict_value("3.14"), "3.14");
        assert_eq!(render_dict_value("hello"), "\"hello\"");
        assert_eq!(render_dict_value("007"), "\"007\"");
        assert_eq!(render_dict_value(""), "\"\"");
        assert_eq!(render_dict_value("-42"), "-42");
    }

    #[test]
    fn json_escape_basic() {
        assert_eq!(json_escape("hi"), "hi");
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("a\nb"), "a\\nb");
    }
}
