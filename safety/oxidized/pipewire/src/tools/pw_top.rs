// pw-top: real-time PipeWire performance viewer.

use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("pw-top");
    let args = expand_short_clusters(raw_args, &['h', 'V', 'b']);
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => break,
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "-V" | "--version" => {
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
            s if s.starts_with("--batch-mode=") => {
                eprintln!("{argv0}: option '--batch-mode' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            "-b" | "--batch-mode" => {}
            opt @ ("-n" | "--iterations") => {
                if i + 1 >= args.len() {
                    if opt == "--iterations" {
                        eprintln!("{argv0}: option '--iterations' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'n'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
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
                    return u8::MAX as i32;
                }
                i += 2;
                continue;
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
                return u8::MAX as i32;
            }
            _ => {}
        }
        i += 1;
    }
    // C connects to the daemon to start the curses UI. Try connecting;
    // emit C's "Can't connect" error if it fails (capital C, matches
    // pw-top.c's show_error).
    let env_remote = std::env::var("PIPEWIRE_REMOTE")
        .ok()
        .filter(|s| !s.is_empty());
    let connect = if let Some(name) = &env_remote {
        let runtime = std::env::var("PIPEWIRE_RUNTIME_DIR")
            .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let path = std::path::PathBuf::from(runtime).join(name);
        crate::pipewire_lib::client::Client::connect_path(&path)
    } else {
        crate::pipewire_lib::client::Client::connect_default()
    };
    match connect {
        Ok(_) => 0,
        Err(_) => {
            eprintln!(
                "Can't connect: {}",
                crate::tools::common::connect_failure_msg_for(env_remote.as_deref())
            );
            255
        }
    }
}

fn print_help(argv0: &str) {
    // pw-top has its own format with a "Usage:" header and tab-aligned
    // batch-mode option. Bytes match `src/tools/pw-top.c::show_help`.
    println!("Usage:");
    println!("{argv0} [options]");
    println!();
    println!("Options:");
    println!("  -b, --batch-mode\t\t         run in non-interactive batch mode");
    println!("  -n, --iterations = NUMBER             exit after NUMBER batch iterations");
    println!("  -r, --remote                          Remote daemon name");
    println!();
    println!("  -h, --help                            Show this help");
    println!("  -V  --version                         Show version");
}
