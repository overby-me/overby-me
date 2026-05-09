// pw-metadata: read/write PipeWire metadata.
//
// Phase 7 partial: --list (walks the registry and prints each Metadata
// global's name), --help, --version. Property reads/writes via the
// Metadata interface require the Metadata.AddListener round-trip and
// will be added when needed.

use crate::pipewire_lib::client::{Client, RegistryGlobal};
use crate::pipewire_lib::interfaces;
use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-metadata");

    let mut list = false;
    let mut name: Option<String> = None;
    let mut remote: Option<String> = None;
    let mut positional: Vec<&str> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "--version" => {
                print_version(argv0);
                return 0;
            }
            "-l" | "--list" => list = true,
            "-n" | "--name" => {
                if let Some(v) = args.get(i + 1) {
                    name = Some(v.clone());
                    i += 2;
                    continue;
                }
                eprintln!("{argv0}: --name needs an argument");
                return 2;
            }
            "-r" | "--remote" => {
                if let Some(v) = args.get(i + 1) {
                    remote = Some(v.clone());
                    i += 2;
                    continue;
                }
                eprintln!("{argv0}: --remote needs an argument");
                return 2;
            }
            "-m" | "--monitor" | "-d" | "--delete" => {}
            s if s.starts_with('-') => {
                eprintln!("{argv0}: unknown flag {s}");
                return 2;
            }
            s => positional.push(s),
        }
        i += 1;
    }

    let _ = positional;

    let globals = match collect_globals(remote.as_deref(), "rust-pipewire-meta") {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{argv0}: {e}");
            return 1;
        }
    };

    // Iterate Metadata globals in registry order. Filter by --name when
    // given. C tool prints a single line per match.
    for g in globals
        .iter()
        .filter(|g| g.interface == interfaces::TYPE_METADATA)
    {
        let meta_name = g
            .props
            .iter()
            .find(|p| p.key == "metadata.name")
            .map(|p| p.value.as_str());
        let meta_name = match meta_name {
            Some(n) => n,
            None => continue,
        };
        if let Some(filter) = name.as_deref()
            && filter != meta_name
        {
            continue;
        }
        println!("Found \"{}\" metadata {}", meta_name, g.id);
        if list {
            continue;
        }
        // Without --list and with no read/write/delete args, the C tool
        // would bind this metadata, subscribe to property events, and run
        // forever. We don't yet implement that flow, so stop after the
        // first match (matching upstream's "Multiple metadata: ignoring..."
        // single-pick semantics for the non-list case).
        break;
    }
    0
}

fn collect_globals(
    remote: Option<&str>,
    app_name: &str,
) -> Result<Vec<RegistryGlobal>, String> {
    let mut client = match remote {
        Some(name) if name.starts_with('/') => {
            Client::connect_path(std::path::Path::new(name))
        }
        Some(name) => {
            let runtime = std::env::var("XDG_RUNTIME_DIR")
                .map_err(|_| "XDG_RUNTIME_DIR unset".to_string())?;
            Client::connect_path(&std::path::PathBuf::from(runtime).join(name))
        }
        None => Client::connect_default(),
    }
    .map_err(|e| format!("connect: {e}"))?;

    client
        .handshake(app_name)
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
        if msg.opcode == interfaces::registry_event::GLOBAL && msg.id == 2
            && let Ok(g) = crate::pipewire_lib::client::decode_registry_global(&msg.args)
        {
            globals.push(g);
        }
        if msg.id == interfaces::ID_CORE
            && msg.opcode == interfaces::core_event::DONE
            && let Ok((_id, seq)) =
                crate::pipewire_lib::client::decode_core_done(&msg.args)
            && seq == sync_seq
        {
            break;
        }
    }
    Ok(globals)
}

fn print_help(argv0: &str) {
    println!("{argv0} [options] [ id [ key [ value [ type ] ] ] ]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -r, --remote                          Remote daemon name");
    println!("  -l, --list                            List available metadata");
    println!("  -m, --monitor                         Monitor metadata");
    println!("  -d, --delete                          Delete metadata");
    println!("  -n, --name                            Metadata name (default: \"default\")");
}
