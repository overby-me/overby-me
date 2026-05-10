// pw-metadata: read/write PipeWire metadata.
//
// Phase 7 partial: --list (walks the registry and prints each Metadata
// global's name), --help, --version. Property reads/writes via the
// Metadata interface require the Metadata.AddListener round-trip and
// will be added when needed.

use crate::pipewire_lib::client::{Client, RegistryGlobal};
use crate::pipewire_lib::interfaces;
use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("pw-metadata");
    let args = expand_short_clusters(raw_args, &['h', 'V', 'l', 'm', 'd']);

    let mut list = false;
    let mut name: Option<String> = None;
    let mut remote: Option<String> = None;
    let mut positional: Vec<&str> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => {
                for v in args.iter().skip(i + 1) {
                    positional.push(v.as_str());
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
            s if s.starts_with("--monitor=")
                || s.starts_with("--delete=")
                || s.starts_with("--list=") =>
            {
                let name = s.split_once('=').map(|(n, _)| n).unwrap_or(s);
                eprintln!("{argv0}: option '{name}' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            "-l" | "--list" => list = true,
            opt @ ("-n" | "--name") => {
                if i + 1 >= args.len() {
                    if opt == "--name" {
                        eprintln!("{argv0}: option '--name' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'n'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
                name = Some(args[i + 1].clone());
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
            "-m" | "--monitor" | "-d" | "--delete" => {}
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
                return u8::MAX as i32;
            }
            s => positional.push(s),
        }
        i += 1;
    }

    let _ = positional;

    let globals = match collect_globals(remote.as_deref(), "rust-pipewire-meta") {
        Ok(g) => g,
        Err(e) => {
            // C pw-metadata uses pw-link / pw-dump's `can't connect: %m`
            // pattern (no `Error:` wrapper), with errno mapped through
            // EHOSTDOWN.
            if e.contains("connect:") || e.starts_with("connect:") {
                eprintln!("can't connect: {}", crate::tools::common::connect_failure_msg());
            } else {
                eprintln!("{argv0}: {e}");
            }
            return 255;
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

fn collect_globals(remote: Option<&str>, app_name: &str) -> Result<Vec<RegistryGlobal>, String> {
    // PIPEWIRE_REMOTE supplies the socket name when -r wasn't given.
    let env_remote = std::env::var("PIPEWIRE_REMOTE").ok().filter(|s| !s.is_empty());
    let chosen: Option<String> = remote.map(String::from).or(env_remote);
    let mut client = match chosen.as_deref() {
        Some(name) if name.starts_with('/') => Client::connect_path(std::path::Path::new(name)),
        Some(name) => {
            let runtime = std::env::var("PIPEWIRE_RUNTIME_DIR")
                .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
                .unwrap_or_else(|_| "/tmp".to_string());
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
        if msg.opcode == interfaces::registry_event::GLOBAL
            && msg.id == 2
            && let Ok(g) = crate::pipewire_lib::client::decode_registry_global(&msg.args)
        {
            globals.push(g);
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
