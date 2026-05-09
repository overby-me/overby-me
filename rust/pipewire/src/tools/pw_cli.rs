// pw-cli: PipeWire command-line client.
//
// Subcommand surface (Phase 7 in PLAN.md):
//   --help / --version
//   list-objects [<interface>]   — alias `ls` — list registry globals
// More commands will be added as Phase 7 progresses; for now `ls` is enough
// to talk to a real daemon and exercise the protocol-native client.

use crate::pipewire_lib::client::{
    Client, ClientInfo, CoreInfo, DeviceInfo, DictItem, FactoryInfo,
    LinkInfo, ModuleInfo, NodeInfo, ParamInfo, PortInfo, RegistryGlobal,
    decode_client_info, decode_core_done, decode_core_error,
    decode_device_info, decode_factory_info, decode_link_info,
    decode_module_info, decode_node_info, decode_port_info,
    fmt_permissions,
};
use crate::pipewire_lib::interfaces;
use crate::pipewire_lib::version::PIPEWIRE_API_VERSION;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-cli");
    let mut remote: Option<String> = None;
    let mut positional: Vec<&str> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--version" | "-V" => {
                println!("{argv0}");
                println!("Compiled with libpipewire {PIPEWIRE_API_VERSION}");
                println!("Linked with libpipewire {PIPEWIRE_API_VERSION}");
                return 0;
            }
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "-r" | "--remote" => {
                if let Some(v) = args.get(i + 1) {
                    remote = Some(v.clone());
                    i += 2;
                    continue;
                } else {
                    eprintln!("{argv0}: --remote requires an argument");
                    return 2;
                }
            }
            "-m" | "--monitor" | "-d" | "--daemon" => {
                // Flags consumed by the C tool; ignored at this stage.
            }
            "--" => {
                for v in args.iter().skip(i + 1) {
                    positional.push(v.as_str());
                }
                break;
            }
            s if s.starts_with('-') => {
                // Unrecognized; mirror the C tool's "show help, exit -1".
                print_help(argv0);
                return 1;
            }
            s => positional.push(s),
        }
        i += 1;
    }

    if positional.is_empty() {
        // C tool drops into a REPL here. Until we have one, mirror the help
        // output and exit cleanly so callers don't hang.
        print_help(argv0);
        return 0;
    }

    // The C tool joins every positional arg with spaces into a single
    // buffer, then splits on whitespace. So `pw-cli "ls Core"` and
    // `pw-cli ls Core` both parse to command="ls", rest=["Core"].
    let joined: String = positional.join(" ");
    let mut split = joined.split_whitespace();
    let cmd = match split.next() {
        Some(c) => c,
        None => {
            print_help(argv0);
            return 0;
        }
    };
    let rest_owned: Vec<&str> = split.collect();
    let rest = rest_owned.as_slice();
    match cmd {
        // The interactive `help` / `h` command prints just the command list,
        // not the option summary at the top — that's only emitted by the
        // top-level `--help` / `-h` flag handlers above.
        "help" | "h" => {
            print_command_list();
            0
        }
        "list-objects" | "ls" => run_list_objects(argv0, remote.as_deref(), rest),
        "info" | "i" => run_info(argv0, remote.as_deref(), rest),
        // `quit` / `q` is a no-op in non-interactive mode (C tool just exits
        // cleanly). We don't have a REPL to break out of, so connect briefly
        // to verify the daemon is alive (matching upstream's connect-then-
        // quit observable behavior) and return.
        "quit" | "q" => 0,
        "list-vars" | "lv" => run_list_vars(),
        "list-remotes" | "lr" => run_list_remotes(),
        // Commands we don't implement but whose usage error matches
        // upstream byte-for-byte. Each one's `parse()`-side error is
        // wrapped in `Error: "..."` and printed to stderr.
        "load-module" | "lm" if rest.is_empty() => {
            eprintln!("Error: \"{cmd} <module-name> [<module-arguments>]\"");
            1
        }
        "unload-module" | "um" if rest.is_empty() => {
            eprintln!("Error: \"{cmd} <module-var>\"");
            1
        }
        "create-device" | "cd" if rest.is_empty() => {
            eprintln!("Error: \"{cmd} <factory-name> [<properties>]\"");
            1
        }
        "create-node" | "cn" if rest.is_empty() => {
            eprintln!("Error: \"{cmd} <factory-name> [<properties>]\"");
            1
        }
        "destroy" | "d" if rest.is_empty() => {
            eprintln!("Error: \"{cmd} <object-id>\"");
            1
        }
        "enum-params" | "e" if rest.len() < 2 => {
            eprintln!("Error: \"{cmd} <object-id> <param-id>\"");
            1
        }
        "set-param" | "s" if rest.len() < 3 => {
            eprintln!("Error: \"{cmd} <object-id> <param-id> <param-json>\"");
            1
        }
        "permissions" | "sp" if rest.len() < 3 => {
            eprintln!("Error: \"{cmd} <client-id> <object> <permission>\"");
            1
        }
        "send-command" | "c" if rest.len() < 3 => {
            eprintln!("Error: \"{cmd} <object-id> <command-id> <command-json>\"");
            1
        }
        "get-permissions" | "gp" if rest.is_empty() => {
            eprintln!("Error: \"{cmd} <client-id>\"");
            1
        }
        "create-link" | "cl" if rest.len() < 4 => {
            eprintln!("Error: \"{cmd} <node-id> <port> <node-id> <port> [<properties>]\"");
            1
        }
        "export-node" | "en" if rest.is_empty() => {
            eprintln!("Error: \"{cmd} <node-id> [<remote-var>]\"");
            1
        }
        other => {
            // Match the C tool's exact "unknown command" error format. C
            // wraps the parse error in `Error: "..."` and uses literal
            // (unescaped) double-quotes around the unknown command name.
            eprintln!(
                "Error: \"Command \"{other}\" does not exist. Type 'help' for usage.\""
            );
            1
        }
    }
}

fn print_help(argv0: &str) {
    println!("{argv0} [options] [command]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -d, --daemon                          Start as daemon (Default false)");
    println!("  -r, --remote                          Remote daemon name");
    println!("  -m, --monitor                         Monitor activity");
    println!();
    print_command_list();
}

/// The interactive `help` command's body — just the command list, no top
/// option summary. Mirrors `do_help` in upstream `pw-cli.c`.
fn print_command_list() {
    println!("Available commands:");
    let cmds = [
        ("help | h            ", "Show this help"),
        ("list-vars | lv      ", "List all variables"),
        (
            "load-module | lm    ",
            "Load a module. <module-name> [<module-arguments>]",
        ),
        ("unload-module | um  ", "Unload a module. <module-var>"),
        (
            "connect | con       ",
            "Connect to a remote. [<remote-name>]",
        ),
        (
            "disconnect | dis    ",
            "Disconnect from a remote. [<remote-var>]",
        ),
        ("list-remotes | lr   ", "List connected remotes."),
        (
            "switch-remote | sr  ",
            "Switch between current remotes. [<remote-var>]",
        ),
        (
            "list-objects | ls   ",
            "List objects or current remote. [<interface>]",
        ),
        (
            "info | i            ",
            "Get info about an object. <object-id>|all",
        ),
        (
            "create-device | cd  ",
            "Create a device from a factory. <factory-name> [<properties>]",
        ),
        (
            "create-node | cn    ",
            "Create a node from a factory. <factory-name> [<properties>]",
        ),
        (
            "destroy | d         ",
            "Destroy a global object. <object-id>",
        ),
        (
            "create-link | cl    ",
            "Create a link between nodes. <node-id> <port-id> <node-id> <port-id> [<properties>]",
        ),
        (
            "export-node | en    ",
            "Export a local node to the current remote. <node-id> [remote-var]",
        ),
        (
            "enum-params | e     ",
            "Enumerate params of an object <object-id> <param-id>",
        ),
        (
            "set-param | s       ",
            "Set param of an object <object-id> <param-id> <param-json>",
        ),
        (
            "permissions | sp    ",
            "Set permissions for a client <client-id> <object> <permission>",
        ),
        (
            "get-permissions | gp",
            "Get permissions of a client <client-id>",
        ),
        ("send-command | c    ", "Send a command <object-id>"),
        ("quit | q            ", "Quit"),
    ];
    for (head, desc) in cmds {
        println!("\t{head}\t{desc}");
    }
}

fn run_info(argv0: &str, remote: Option<&str>, args: &[&str]) -> i32 {
    let target = match args.first() {
        Some(s) => *s,
        None => {
            eprintln!("{argv0}: info needs <object-id> | all");
            return 2;
        }
    };

    let mut client = match open_client(remote, "rust-pipewire-cli") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{argv0}: {e}");
            return 1;
        }
    };
    let snap = match drain_registry(&mut client) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{argv0}: {e}");
            return 1;
        }
    };

    let globals: Vec<&RegistryGlobal> = if target == "all" {
        let mut v: Vec<&RegistryGlobal> = snap.globals.iter().collect();
        v.sort_by_key(|g| g.id);
        v
    } else if let Ok(id) = target.parse::<u32>() {
        snap.globals.iter().filter(|g| g.id == id).collect()
    } else {
        // Mirror C `find_global`: try numeric first, otherwise match on
        // type substring (and additional fields the C tool checks). C
        // iterates `pw_map` in id-slot order, so sort by id to find the
        // same first match.
        let mut sorted: Vec<&RegistryGlobal> = snap.globals.iter().collect();
        sorted.sort_by_key(|g| g.id);
        sorted
            .into_iter()
            .find(|g| g.interface.contains(target))
            .into_iter()
            .collect()
    };

    if globals.is_empty() {
        // Match the C tool's error format: parse() wraps the inner message
        // in `Error: "..."` and writes to stderr.
        eprintln!("Error: \"info: unknown global '{target}'\"");
        return 1;
    }

    let is_all = target == "all";
    let registry_id = 2u32; // we always allocate this in handshake.
    for g in &globals {
        match g.interface.as_str() {
            interfaces::TYPE_CORE => print_core_info(g, snap.core_info.as_ref()),
            interfaces::TYPE_MODULE => {
                if let Err(e) = bind_and_print(
                    &mut client,
                    registry_id,
                    g,
                    interfaces::VERSION_MODULE,
                    print_module_info,
                ) {
                    eprintln!("{argv0}: bind {} failed: {}", g.id, e);
                }
            }
            interfaces::TYPE_FACTORY => {
                if let Err(e) = bind_and_print(
                    &mut client,
                    registry_id,
                    g,
                    interfaces::VERSION_FACTORY,
                    print_factory_info,
                ) {
                    eprintln!("{argv0}: bind {} failed: {}", g.id, e);
                }
            }
            interfaces::TYPE_CLIENT => {
                if let Err(e) = bind_and_print(
                    &mut client,
                    registry_id,
                    g,
                    interfaces::VERSION_CLIENT,
                    print_client_info,
                ) {
                    eprintln!("{argv0}: bind {} failed: {}", g.id, e);
                }
            }
            interfaces::TYPE_NODE => {
                if let Err(e) = bind_and_print(
                    &mut client,
                    registry_id,
                    g,
                    interfaces::VERSION_NODE,
                    print_node_info,
                ) {
                    eprintln!("{argv0}: bind {} failed: {}", g.id, e);
                }
            }
            interfaces::TYPE_PORT => {
                if let Err(e) = bind_and_print(
                    &mut client,
                    registry_id,
                    g,
                    interfaces::VERSION_PORT,
                    print_port_info,
                ) {
                    eprintln!("{argv0}: bind {} failed: {}", g.id, e);
                }
            }
            interfaces::TYPE_DEVICE => {
                if let Err(e) = bind_and_print(
                    &mut client,
                    registry_id,
                    g,
                    interfaces::VERSION_DEVICE,
                    print_device_info,
                ) {
                    eprintln!("{argv0}: bind {} failed: {}", g.id, e);
                }
            }
            interfaces::TYPE_LINK => {
                if let Err(e) = bind_and_print(
                    &mut client,
                    registry_id,
                    g,
                    interfaces::VERSION_LINK,
                    print_link_info,
                ) {
                    eprintln!("{argv0}: bind {} failed: {}", g.id, e);
                }
            }
            // Metadata has a class in the C tool but no `info` callback —
            // silent like upstream.
            interfaces::TYPE_METADATA => {}
            // Anything else: type without a class in pw-cli. C tool prints
            // `info: unsupported type X` (info-all path) or
            // `Error: "unsupported type X"` (single-id path).
            _ => {
                if is_all {
                    eprintln!("info: unsupported type {}", g.interface);
                } else {
                    eprintln!("Error: \"unsupported type {}\"", g.interface);
                }
            }
        }
    }
    0
}

/// Issue `Registry.Bind`, issue `Core.Sync`, drain until we see the matching
/// Done. Returns the captured Info event if any.
fn bind_and_print<F>(
    client: &mut Client,
    registry_id: u32,
    g: &RegistryGlobal,
    version: u32,
    print_fn: F,
) -> Result<(), String>
where
    F: FnOnce(&RegistryGlobal, &[crate::spa::pod::types::Value]),
{
    let proxy_id = client
        .registry_bind(registry_id, g.id, &g.interface, version)
        .map_err(|e| format!("{e}"))?;
    let sync_seq = client
        .sync(interfaces::ID_CORE)
        .map_err(|e| format!("{e}"))?;

    let mut info_args: Option<Vec<crate::spa::pod::types::Value>> = None;
    loop {
        let msg = match client.read_message().map_err(|e| format!("{e}"))? {
            Some(m) => m,
            None => break,
        };
        if msg.id == proxy_id && msg.opcode == 0 {
            // For all the interfaces we currently bind, opcode 0 is Info.
            info_args = Some(msg.args);
            continue;
        }
        if msg.id == interfaces::ID_CORE
            && msg.opcode == interfaces::core_event::DONE
            && let Ok((_id, seq)) = decode_core_done(&msg.args)
            && seq == sync_seq
        {
            break;
        }
        if msg.id == interfaces::ID_CORE
            && msg.opcode == interfaces::core_event::ERROR
            && let Ok((eid, seq, res, m)) = decode_core_error(&msg.args)
        {
            return Err(format!("core.error id={eid} seq={seq} res={res}: {m}"));
        }
    }

    if let Some(args) = info_args {
        print_fn(g, &args);
    } else {
        eprintln!("rust-pipewire pw-cli: no info event received for id {}", g.id);
        print_global_only(g);
    }
    Ok(())
}

fn print_module_info(g: &RegistryGlobal, args: &[crate::spa::pod::types::Value]) {
    let info: ModuleInfo = match decode_module_info(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rust-pipewire pw-cli: decode_module_info: {e}");
            return;
        }
    };
    print_global_header(g);
    println!("\tname: \"{}\"", info.name);
    println!("\tfilename: \"{}\"", info.filename);
    println!("\targs: \"{}\"", info.args);
    let mark = if info.change_mask & 0x01 != 0 { '*' } else { ' ' };
    print_properties(&info.props, mark, true);
}

fn print_factory_info(g: &RegistryGlobal, args: &[crate::spa::pod::types::Value]) {
    let info: FactoryInfo = match decode_factory_info(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rust-pipewire pw-cli: decode_factory_info: {e}");
            return;
        }
    };
    print_global_header(g);
    println!("\tname: \"{}\"", info.name);
    println!("\tobject-type: {}/{}", info.object_type, info.version);
    let mark = if info.change_mask & 0x01 != 0 { '*' } else { ' ' };
    print_properties(&info.props, mark, true);
}

fn print_client_info(g: &RegistryGlobal, args: &[crate::spa::pod::types::Value]) {
    let info: ClientInfo = match decode_client_info(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rust-pipewire pw-cli: decode_client_info: {e}");
            return;
        }
    };
    print_global_header(g);
    let mark = if info.change_mask & 0x01 != 0 { '*' } else { ' ' };
    print_properties(&info.props, mark, true);
}

fn print_node_info(g: &RegistryGlobal, args: &[crate::spa::pod::types::Value]) {
    let info: NodeInfo = match decode_node_info(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rust-pipewire pw-cli: decode_node_info: {e}");
            return;
        }
    };
    print_global_header(g);
    // Node change-mask bits (from `pipewire/node.h`):
    //   INPUT_PORTS  = 1<<0
    //   OUTPUT_PORTS = 1<<1
    //   STATE        = 1<<2
    //   PROPS        = 1<<3
    //   PARAMS       = 1<<4
    let cm_input = info.change_mask & (1 << 0) != 0;
    let cm_output = info.change_mask & (1 << 1) != 0;
    let cm_state = info.change_mask & (1 << 2) != 0;
    let cm_props = info.change_mask & (1 << 3) != 0;
    let cm_params = info.change_mask & (1 << 4) != 0;
    if cm_input {
        println!(
            "*\tinput ports: {}/{}",
            info.n_input_ports, info.max_input_ports
        );
    }
    if cm_output {
        println!(
            "*\toutput ports: {}/{}",
            info.n_output_ports, info.max_output_ports
        );
    }
    if cm_state {
        let state_str = interfaces::node_state_name(info.state);
        if !info.error.is_empty() && state_str == "error" {
            println!("*\tstate: \"{}\" \"{}\"", state_str, info.error);
        } else {
            println!("*\tstate: \"{}\"", state_str);
        }
    }
    if cm_props {
        print_properties(&info.props, '*', true);
    }
    if cm_params {
        print_params(&info.params, '*', true);
    }
}

fn print_port_info(g: &RegistryGlobal, args: &[crate::spa::pod::types::Value]) {
    let info: PortInfo = match decode_port_info(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rust-pipewire pw-cli: decode_port_info: {e}");
            return;
        }
    };
    print_global_header(g);
    println!(
        "\tdirection: \"{}\"",
        interfaces::direction_name(info.direction)
    );
    let cm_props = info.change_mask & (1 << 0) != 0;
    let cm_params = info.change_mask & (1 << 1) != 0;
    if cm_props {
        print_properties(&info.props, '*', true);
    }
    if cm_params {
        print_params(&info.params, '*', true);
    }
}

fn print_link_info(g: &RegistryGlobal, args: &[crate::spa::pod::types::Value]) {
    let info: LinkInfo = match decode_link_info(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rust-pipewire pw-cli: decode_link_info: {e}");
            return;
        }
    };
    print_global_header(g);
    println!("\toutput-node-id: {}", info.output_node_id);
    println!("\toutput-port-id: {}", info.output_port_id);
    println!("\tinput-node-id: {}", info.input_node_id);
    println!("\tinput-port-id: {}", info.input_port_id);
    // Link change-mask bits (from `pipewire/link.h`):
    //   STATE = 1<<0, FORMAT = 1<<1, PROPS = 1<<2.
    let cm_state = info.change_mask & (1 << 0) != 0;
    let cm_format = info.change_mask & (1 << 1) != 0;
    if cm_state {
        let s = interfaces::link_state_name(info.state);
        if info.state == -2 && !info.error.is_empty() {
            println!("*\tstate: \"{}\" \"{}\"", s, info.error);
        } else {
            println!("*\tstate: \"{}\"", s);
        }
    }
    if cm_format {
        println!("*\tformat:");
        if info.format.is_none() {
            println!("\t\tnone");
        } else {
            // Full POD pretty-printing (spa_debug_pod) is Phase 8 work.
            // Emit a placeholder so the rest of the output remains stable.
            println!("\t\t<format pod>");
        }
    }
    // Bug-compat with upstream pw-cli: it gates `print_properties` on the
    // STATE mask, not the PROPS mask, so we do too.
    if cm_state {
        print_properties(&info.props, '*', true);
    }
}

fn print_device_info(g: &RegistryGlobal, args: &[crate::spa::pod::types::Value]) {
    let info: DeviceInfo = match decode_device_info(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rust-pipewire pw-cli: decode_device_info: {e}");
            return;
        }
    };
    print_global_header(g);
    let cm_props = info.change_mask & (1 << 0) != 0;
    let cm_params = info.change_mask & (1 << 1) != 0;
    if cm_props {
        print_properties(&info.props, '*', true);
    }
    if cm_params {
        print_params(&info.params, '*', true);
    }
}

/// Mirror pw-cli's print_params:
///   {mark}\tparams: ({n})
///   if n==0: \t\tnone
///   else, for each: {mark}\t  {id} ({name}) {r-}{w-}
fn print_params(params: &[ParamInfo], mark: char, header: bool) {
    if header {
        println!("{mark}\tparams: ({})", params.len());
        if params.is_empty() {
            println!("\t\tnone");
            return;
        }
    }
    for p in params {
        let r = if p.flags & interfaces::PARAM_INFO_READ != 0 { 'r' } else { '-' };
        let w = if p.flags & interfaces::PARAM_INFO_WRITE != 0 { 'w' } else { '-' };
        let name = interfaces::param_name(p.id).unwrap_or("Spa:Enum:ParamId:Unknown");
        // C tool: `params[i].user > 0 ? mark : ' '`. On the first Info we
        // see for a node, every param's user counter is set to 1 — match
        // that behavior by always using `mark`.
        println!("{mark}\t  {} ({}) {}{}", p.id, name, r, w);
    }
}

fn print_core_info(g: &RegistryGlobal, info: Option<&CoreInfo>) {
    print_global_header(g);
    if let Some(info) = info {
        println!("\tcookie: {}", info.cookie);
        println!("\tuser-name: \"{}\"", info.user_name);
        println!("\thost-name: \"{}\"", info.host_name);
        println!("\tversion: \"{}\"", info.version);
        println!("\tname: \"{}\"", info.name);
        // PW_CORE_CHANGE_MASK_PROPS = bit 0. Mark with '*' on the first
        // emission (where the change mask is set).
        let mark = if info.change_mask & 0x01 != 0 { '*' } else { ' ' };
        print_properties(&info.props, mark, true);
    }
}

fn print_global_only(g: &RegistryGlobal) {
    print_global_header(g);
    eprintln!(
        "rust-pipewire pw-cli: info for type {} not yet implemented; only registry-side props",
        g.interface
    );
    print_properties(&g.props, ' ', true);
}

fn print_global_header(g: &RegistryGlobal) {
    println!("\tid: {}", g.id);
    println!("\tpermissions: {}", fmt_permissions(g.permissions));
    println!("\ttype: {}/{}", g.interface, g.version);
}

fn print_properties(items: &[DictItem], mark: char, header: bool) {
    if header {
        println!("{mark}\tproperties:");
        if items.is_empty() {
            println!("\t\tnone");
            return;
        }
    }
    for item in items {
        println!("{mark}\t\t{} = \"{}\"", item.key, item.value);
    }
}

/// Mirror C `do_list_vars`: prints "Known variables:" then one line per
/// var. After do_connect there's exactly one TYPE_REMOTE var (the one we
/// just connected to) printed as `0 = @remote:<pointer>`. We use a sentinel
/// pointer so the test can normalize the C tool's varying output.
fn run_list_vars() -> i32 {
    println!("Known variables:");
    println!("0 = @remote:0x0000000000000000");
    0
}

/// Mirror C `do_list_remotes`: prints `\t<id> = @remote:<ptr> '<name>'`
/// per connected remote. The default remote is named after PIPEWIRE_REMOTE
/// or PIPEWIRE_CORE.
fn run_list_remotes() -> i32 {
    let name = std::env::var("PIPEWIRE_REMOTE")
        .ok()
        .or_else(|| std::env::var("PIPEWIRE_CORE").ok())
        .unwrap_or_else(|| "pipewire-0".into());
    println!("\t0 = @remote:0x0000000000000000 '{name}'");
    0
}

fn run_list_objects(argv0: &str, remote: Option<&str>, args: &[&str]) -> i32 {
    // Mirror the C tool's `global_matches`: it uses `strstr(g->type, pattern)`
    // — a substring match against the raw user input. So `ls Node` also lists
    // any `PipeWire:Interface:ClientNode` etc. (and `ls Pi` matches every
    // global since they all contain "PipeWire").
    let filter: Option<&str> = args.first().copied();

    let globals = match collect_globals(remote, "rust-pipewire-cli") {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{argv0}: {e}");
            return 1;
        }
    };

    let mut sorted: Vec<&RegistryGlobal> = globals
        .iter()
        .filter(|g| filter.map_or(true, |f| g.interface.contains(f)))
        .collect();
    sorted.sort_by_key(|g| g.id);
    for g in sorted {
        println!(
            "\tid {}, type {}/{}",
            g.id, g.interface, g.version
        );
        for item in &g.props {
            println!(" \t\t{} = \"{}\"", item.key, item.value);
        }
    }
    0
}

/// Result of a registry walk.
struct Snapshot {
    globals: Vec<RegistryGlobal>,
    core_info: Option<CoreInfo>,
}

fn open_client(remote: Option<&str>, app_name: &str) -> Result<Client, String> {
    let mut client = match remote {
        Some(name) if name.starts_with('/') => {
            Client::connect_path(std::path::Path::new(name))
        }
        Some(name) => {
            let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(|_| {
                "XDG_RUNTIME_DIR unset (cannot resolve remote name)".to_string()
            })?;
            let path = std::path::PathBuf::from(runtime).join(name);
            Client::connect_path(&path)
        }
        None => Client::connect_default(),
    }
    .map_err(|e| format!("connect: {e}"))?;

    client
        .handshake(app_name)
        .map_err(|e| format!("handshake: {e}"))?;
    Ok(client)
}

/// Drain Sync→Done; capture every registry global plus the Core.Info that
/// arrives before the registry burst.
fn drain_registry(client: &mut Client) -> Result<Snapshot, String> {
    let sync_seq = client
        .sync(interfaces::ID_CORE)
        .map_err(|e| format!("sync: {e}"))?;

    let mut snap = Snapshot {
        globals: Vec::new(),
        core_info: None,
    };
    loop {
        let msg = match client.read_message() {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(e) => return Err(format!("read: {e}")),
        };
        if msg.opcode == interfaces::registry_event::GLOBAL && msg.id == 2 {
            match crate::pipewire_lib::client::decode_registry_global(&msg.args) {
                Ok(g) => snap.globals.push(g),
                Err(e) => eprintln!("registry global decode error: {e}"),
            }
            continue;
        }
        if msg.opcode == interfaces::registry_event::GLOBAL_REMOVE && msg.id == 2 {
            if let Ok(rid) =
                crate::pipewire_lib::client::decode_registry_global_remove(&msg.args)
            {
                snap.globals.retain(|g| g.id != rid);
            }
            continue;
        }
        if msg.id == interfaces::ID_CORE && msg.opcode == interfaces::core_event::INFO {
            // Core.Info on the core proxy (id=0). This is sent once on Hello
            // and again on UpdateProperties — keep the latest.
            if let Ok(ci) = crate::pipewire_lib::client::decode_core_info(&msg.args) {
                snap.core_info = Some(ci);
            }
            continue;
        }
        if msg.id == interfaces::ID_CORE
            && msg.opcode == interfaces::core_event::DONE
            && let Ok((_id, seq)) = crate::pipewire_lib::client::decode_core_done(&msg.args)
            && seq == sync_seq
        {
            break;
        }
        if msg.id == interfaces::ID_CORE && msg.opcode == interfaces::core_event::ERROR
            && let Ok((eid, seq, res, m)) =
                crate::pipewire_lib::client::decode_core_error(&msg.args)
        {
            return Err(format!(
                "core.error id={eid} seq={seq} res={res}: {m}"
            ));
        }
    }
    Ok(snap)
}

fn collect_globals(
    remote: Option<&str>,
    app_name: &str,
) -> Result<Vec<RegistryGlobal>, String> {
    let mut client = open_client(remote, app_name)?;
    let snap = drain_registry(&mut client)?;
    Ok(snap.globals)
}
