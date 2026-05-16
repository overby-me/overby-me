// pw-dump: dump a running PipeWire registry as JSON.
//
// For each registry global we also bind a proxy and read its Info event,
// which lets us emit the per-interface "info" block the C tool produces
// (Module name/filename/args, Factory name/type/version, Client props,
// Node ports+state+params, Device/Port/Link details). Core.Info is
// captured during handshake and attached to the Core global.

use crate::pipewire_lib::client::{
    Client, ClientInfo, CoreInfo, DeviceInfo, FactoryInfo, LinkInfo, ModuleInfo, NodeInfo,
    PortInfo, RegistryGlobal,
};
use crate::pipewire_lib::interfaces;
use crate::tools::common::{expand_short_clusters, print_version};

/// Per-interface Info event payload for a bound global.
#[derive(Debug, Clone)]
enum InfoData {
    Module(ModuleInfo),
    Factory(FactoryInfo),
    Client(ClientInfo),
    Node(NodeInfo),
    Port(PortInfo),
    Device(DeviceInfo),
    Link(LinkInfo),
    Core(CoreInfo),
}

/// Registry global plus the (optional) bound-proxy Info event.
struct GlobalEntry {
    global: RegistryGlobal,
    info: Option<InfoData>,
}

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
                return u8::MAX as i32;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with("--no-colors=")
                || s.starts_with("--raw=")
                || s.starts_with("--spa=")
                || s.starts_with("--monitor=") =>
            {
                let name = s.split_once('=').map(|(n, _)| n).unwrap_or(s);
                eprintln!("{argv0}: option '{name}' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            opt @ ("-i" | "--indent") => {
                if i + 1 >= args.len() {
                    if opt == "--indent" {
                        eprintln!("{argv0}: option '--indent' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'i'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<usize>().ok()) {
                    indent = v;
                }
                i += 2;
                continue;
            }
            s if s.starts_with("--indent=") => {
                if let Ok(v) = s["--indent=".len()..].parse::<usize>() {
                    indent = v;
                }
            }
            s if s.starts_with("-i") && s.len() > 2 => {
                if let Ok(v) = s[2..].parse::<usize>() {
                    indent = v;
                }
            }
            opt @ ("-r" | "--remote") => {
                if i + 1 >= args.len() {
                    if opt == "--remote" {
                        eprintln!("{argv0}: option '--remote' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'r'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
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
                // C: `--color=` (with `=`) sets optarg to "" — getopt
                // distinguishes that from bare `--color` (optarg=NULL).
                // Bare form accepts default; empty value fails the
                // auto/never/always match like any other unknown value.
                let val = &s["--color=".len()..];
                if !matches!(val, "never" | "always" | "auto") {
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
                return u8::MAX as i32;
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with('-') && !s.starts_with("--") => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
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

    let mut sorted: Vec<&GlobalEntry> = globals
        .iter()
        .filter(|e| filter_id.is_none_or(|f| e.global.id == f))
        .filter(|e| {
            filter_iface
                .as_deref()
                .is_none_or(|f| e.global.interface == f)
        })
        .collect();
    sorted.sort_by_key(|e| e.global.id);

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

fn collect_globals(remote: Option<&str>) -> Result<Vec<GlobalEntry>, String> {
    // PIPEWIRE_REMOTE supplies the socket name when -r wasn't given.
    let env_remote = std::env::var("PIPEWIRE_REMOTE").ok();
    let chosen: Option<String> = remote.map(String::from).or(env_remote);
    let mut client = match chosen.as_deref() {
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

    let registry_id = client
        .handshake("rust-pipewire-dump")
        .map_err(|e| format!("handshake: {e}"))?;
    let sync_seq = client
        .sync(interfaces::ID_CORE)
        .map_err(|e| format!("sync: {e}"))?;

    let mut globals: Vec<RegistryGlobal> = Vec::new();
    let mut core_info: Option<CoreInfo> = None;
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
            && msg.opcode == interfaces::core_event::INFO
            && let Ok(ci) = crate::pipewire_lib::client::decode_core_info(&msg.args)
        {
            core_info = Some(ci);
        }
        if msg.id == interfaces::ID_CORE
            && msg.opcode == interfaces::core_event::DONE
            && let Ok((_id, seq)) = crate::pipewire_lib::client::decode_core_done(&msg.args)
            && seq == sync_seq
        {
            break;
        }
    }

    // Bind each global and collect its Info event. We do this sequentially
    // (bind → sync → wait for Info+Done → continue). The C tool issues all
    // binds + syncs up-front and demultiplexes the responses; that's more
    // efficient but adds bookkeeping. Sequential is fine until the
    // registry gets large.
    let entries: Vec<GlobalEntry> = globals
        .into_iter()
        .map(|g| {
            let info = if g.interface == interfaces::TYPE_CORE {
                core_info.clone().map(InfoData::Core)
            } else {
                bind_info(&mut client, registry_id, &g).ok().flatten()
            };
            GlobalEntry { global: g, info }
        })
        .collect();
    Ok(entries)
}

/// Bind the given global as a proxy, drive a sync round-trip, and return
/// the decoded Info event (if the interface has one we know how to
/// decode). Unknown interfaces return None silently; decode errors are
/// also swallowed since the registry-side data is still useful.
fn bind_info(
    client: &mut Client,
    registry_id: u32,
    g: &RegistryGlobal,
) -> Result<Option<InfoData>, String> {
    let version = match g.interface.as_str() {
        interfaces::TYPE_MODULE => interfaces::VERSION_MODULE,
        interfaces::TYPE_FACTORY => interfaces::VERSION_FACTORY,
        interfaces::TYPE_CLIENT => interfaces::VERSION_CLIENT,
        interfaces::TYPE_NODE => interfaces::VERSION_NODE,
        interfaces::TYPE_PORT => interfaces::VERSION_PORT,
        interfaces::TYPE_DEVICE => interfaces::VERSION_DEVICE,
        interfaces::TYPE_LINK => interfaces::VERSION_LINK,
        _ => return Ok(None),
    };

    let proxy_id = client
        .registry_bind(registry_id, g.id, &g.interface, version)
        .map_err(|e| format!("{e}"))?;
    let sync_seq = client
        .sync(interfaces::ID_CORE)
        .map_err(|e| format!("{e}"))?;

    let mut raw_info: Option<Vec<crate::spa::pod::types::Value>> = None;
    while let Some(msg) = client.read_message().map_err(|e| format!("{e}"))? {
        if msg.id == proxy_id && msg.opcode == 0 {
            // Opcode 0 is the Info event for every bindable interface.
            raw_info = Some(msg.args);
            continue;
        }
        if msg.id == interfaces::ID_CORE
            && msg.opcode == interfaces::core_event::DONE
            && let Ok((_id, seq)) = crate::pipewire_lib::client::decode_core_done(&msg.args)
            && seq == sync_seq
        {
            break;
        }
    }

    let args = match raw_info {
        Some(a) => a,
        None => return Ok(None),
    };
    use crate::pipewire_lib::client as cl;
    let info = match g.interface.as_str() {
        interfaces::TYPE_MODULE => cl::decode_module_info(&args).ok().map(InfoData::Module),
        interfaces::TYPE_FACTORY => cl::decode_factory_info(&args).ok().map(InfoData::Factory),
        interfaces::TYPE_CLIENT => cl::decode_client_info(&args).ok().map(InfoData::Client),
        interfaces::TYPE_NODE => cl::decode_node_info(&args).ok().map(InfoData::Node),
        interfaces::TYPE_PORT => cl::decode_port_info(&args).ok().map(InfoData::Port),
        interfaces::TYPE_DEVICE => cl::decode_device_info(&args).ok().map(InfoData::Device),
        interfaces::TYPE_LINK => cl::decode_link_info(&args).ok().map(InfoData::Link),
        _ => None,
    };
    Ok(info)
}

fn write_array(out: &mut String, entries: &[&GlobalEntry], indent: usize) {
    if entries.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push('[');
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_newline_indent(out, indent, 1);
        write_global(out, e, indent, 1);
    }
    push_newline_indent(out, indent, 0);
    out.push(']');
}

fn write_global(out: &mut String, e: &GlobalEntry, indent: usize, level: usize) {
    let g = &e.global;
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
    // C: known interfaces (class table) emit only `info`; unknown
    // interfaces (SecurityContext, Profiler, ClientNode, ...) fall back
    // to registry `props`. Never both.
    if let Some(info) = &e.info {
        out.push(',');
        push_newline_indent(out, indent, level + 1);
        out.push_str("\"info\": ");
        write_info(out, info, indent, level + 1);
    } else {
        out.push(',');
        push_newline_indent(out, indent, level + 1);
        out.push_str("\"props\": ");
        write_props(out, &g.props, indent, level + 1);
    }
    push_newline_indent(out, indent, level);
    out.push('}');
}

fn write_info(out: &mut String, info: &InfoData, indent: usize, level: usize) {
    out.push('{');
    let inner = level + 1;
    match info {
        InfoData::Core(i) => {
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"cookie\": {},", i.cookie));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!(
                "\"user-name\": \"{}\",",
                json_escape(&i.user_name)
            ));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!(
                "\"host-name\": \"{}\",",
                json_escape(&i.host_name)
            ));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"version\": \"{}\",", json_escape(&i.version)));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"name\": \"{}\",", json_escape(&i.name)));
            push_newline_indent(out, indent, inner);
            out.push_str("\"change-mask\": ");
            write_flags(out, i.change_mask as u64, &[(1 << 0, "props")]);
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str("\"props\": ");
            write_props(out, &i.props, indent, inner);
        }
        InfoData::Module(i) => {
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"name\": \"{}\",", json_escape(&i.name)));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"filename\": \"{}\",", json_escape(&i.filename)));
            push_newline_indent(out, indent, inner);
            // C: put_value emits JSON null for a NULL string pointer; our
            // decoder converts POD None into the sentinel "(null)" string
            // (for text-mode parity), so map back to null here.
            out.push_str(&format!("\"args\": {},", json_value_or_null(&i.args)));
            push_newline_indent(out, indent, inner);
            out.push_str("\"change-mask\": ");
            write_flags(out, i.change_mask as u64, &[(1 << 0, "props")]);
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str("\"props\": ");
            write_props(out, &i.props, indent, inner);
        }
        InfoData::Factory(i) => {
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"name\": \"{}\",", json_escape(&i.name)));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"type\": \"{}\",", json_escape(&i.object_type)));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"version\": {},", i.version));
            push_newline_indent(out, indent, inner);
            out.push_str("\"change-mask\": ");
            write_flags(out, i.change_mask as u64, &[(1 << 0, "props")]);
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str("\"props\": ");
            write_props(out, &i.props, indent, inner);
        }
        InfoData::Client(i) => {
            push_newline_indent(out, indent, inner);
            out.push_str("\"change-mask\": ");
            write_flags(out, i.change_mask as u64, &[(1 << 0, "props")]);
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str("\"props\": ");
            write_props(out, &i.props, indent, inner);
        }
        InfoData::Device(i) => {
            push_newline_indent(out, indent, inner);
            out.push_str("\"change-mask\": ");
            write_flags(
                out,
                i.change_mask as u64,
                &[(1 << 0, "props"), (1 << 1, "params")],
            );
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str("\"props\": ");
            write_props(out, &i.props, indent, inner);
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str("\"params\": ");
            write_params(out, &i.params, indent, inner);
        }
        InfoData::Node(i) => {
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"max-input-ports\": {},", i.max_input_ports));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"max-output-ports\": {},", i.max_output_ports));
            push_newline_indent(out, indent, inner);
            out.push_str("\"change-mask\": ");
            write_flags(
                out,
                i.change_mask as u64,
                &[
                    (1 << 0, "input-ports"),
                    (1 << 1, "output-ports"),
                    (1 << 2, "state"),
                    (1 << 3, "props"),
                    (1 << 4, "params"),
                ],
            );
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"n-input-ports\": {},", i.n_input_ports));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"n-output-ports\": {},", i.n_output_ports));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"state\": \"{}\",", node_state_name(i.state)));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"error\": {},", json_null_or_string(&i.error)));
            push_newline_indent(out, indent, inner);
            out.push_str("\"props\": ");
            write_props(out, &i.props, indent, inner);
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str("\"params\": ");
            write_params(out, &i.params, indent, inner);
        }
        InfoData::Port(i) => {
            push_newline_indent(out, indent, inner);
            out.push_str(&format!(
                "\"direction\": \"{}\",",
                if i.direction == 0 { "input" } else { "output" }
            ));
            push_newline_indent(out, indent, inner);
            out.push_str("\"change-mask\": ");
            write_flags(
                out,
                i.change_mask as u64,
                &[(1 << 0, "props"), (1 << 1, "params")],
            );
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str("\"props\": ");
            write_props(out, &i.props, indent, inner);
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str("\"params\": ");
            write_params(out, &i.params, indent, inner);
        }
        InfoData::Link(i) => {
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"output-node-id\": {},", i.output_node_id));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"output-port-id\": {},", i.output_port_id));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"input-node-id\": {},", i.input_node_id));
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"input-port-id\": {},", i.input_port_id));
            push_newline_indent(out, indent, inner);
            out.push_str("\"change-mask\": ");
            write_flags(
                out,
                i.change_mask as u64,
                &[(1 << 0, "state"), (1 << 1, "format"), (1 << 2, "props")],
            );
            out.push(',');
            push_newline_indent(out, indent, inner);
            out.push_str(&format!("\"state\": \"{}\",", link_state_name(i.state)));
            push_newline_indent(out, indent, inner);
            out.push_str("\"format\": null,");
            push_newline_indent(out, indent, inner);
            out.push_str("\"props\": ");
            write_props(out, &i.props, indent, inner);
        }
    }
    push_newline_indent(out, indent, level);
    out.push('}');
}

fn write_flags(out: &mut String, value: u64, flags: &[(u64, &str)]) {
    out.push('[');
    let mut first = true;
    for (mask, name) in flags {
        if value & *mask != 0 {
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

fn write_params(
    out: &mut String,
    params: &[crate::pipewire_lib::client::ParamInfo],
    _indent: usize,
    _level: usize,
) {
    // pw-dump's params include each param's value (enum-params follow-up).
    // We don't yet enumerate params, so emit an empty object — the C tool
    // never emits param values for which info.params[i].flags & READ == 0
    // either, so this matches when no param is readable.
    let _ = params;
    out.push_str("{}");
}

fn node_state_name(s: u32) -> &'static str {
    match s {
        0 => "error",
        1 => "creating",
        2 => "suspended",
        3 => "idle",
        4 => "running",
        _ => "unknown",
    }
}

fn link_state_name(s: i32) -> &'static str {
    match s {
        -2 => "error",
        -1 => "unlinked",
        0 => "init",
        1 => "negotiating",
        2 => "allocating",
        3 => "paused",
        4 => "active",
        _ => "unknown",
    }
}

fn json_null_or_string(s: &str) -> String {
    if s.is_empty() {
        "null".to_string()
    } else {
        format!("\"{}\"", json_escape(s))
    }
}

// Like json_null_or_string, but recognizes the "(null)" sentinel that
// our String-typed decoders use to stand in for a POD None (NULL string
// pointer in C). C's put_value emits unquoted `null` in that case.
fn json_value_or_null(s: &str) -> String {
    if s == "(null)" {
        "null".to_string()
    } else {
        format!("\"{}\"", json_escape(s))
    }
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
    // C's put_dict calls spa_dict_qsort first — keys are emitted in
    // alphabetical order regardless of insertion order. Match that.
    let mut sorted: Vec<&crate::pipewire_lib::client::DictItem> = items.iter().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    out.push('{');
    for (i, item) in sorted.iter().enumerate() {
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
        let e0 = GlobalEntry {
            global: g0,
            info: None,
        };
        let mut s = String::new();
        write_array(&mut s, &[&e0], 2);
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
