// pw-container: namespace/container helper. Real implementation is Phase
// 9-ish (it spawns child processes inside a sandboxed namespace). Help
// text matches upstream byte-for-byte.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    // pw-container passes argv[0] verbatim to its help printer.
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-container");

    let mut remote: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
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
            opt @ ("-P" | "--properties") => {
                if i + 1 >= args.len() {
                    if opt == "--properties" {
                        eprintln!("{argv0}: option '--properties' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'P'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
                i += 2;
                continue;
            }
            s if s.starts_with("--remote=") => {
                remote = Some(s["--remote=".len()..].to_string());
            }
            s if s.starts_with("--properties=") => {}
            s if s.starts_with("-r") && s.len() > 2 => {
                remote = Some(s[2..].to_string());
            }
            s if s.starts_with("-P") && s.len() > 2 => {}
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with('-') => {
                // Cluster `-h<...>` short-circuits to help, `-V<...>` to
                // version (matches getopt's char-by-char processing).
                if s.starts_with("-h") {
                    print_help(argv0);
                    return 0;
                }
                if s.starts_with("-V") {
                    print_version(argv0);
                    return 0;
                }
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            _ => {}
        }
        i += 1;
    }

    // C connects to the daemon to fork an application inside a sandboxed
    // context. Try connecting; emit the same "can't connect" error.
    let env_remote = std::env::var("PIPEWIRE_REMOTE").ok();
    let chosen: Option<String> = remote.or(env_remote);
    let connect = match chosen.as_deref() {
        Some(name) if name.starts_with('/') => {
            crate::pipewire_lib::client::Client::connect_path(std::path::Path::new(name))
        }
        Some(name) => {
            let runtime = std::env::var("PIPEWIRE_RUNTIME_DIR")
                .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
                .unwrap_or_else(|_| "/tmp".to_string());
            crate::pipewire_lib::client::Client::connect_path(
                &std::path::PathBuf::from(runtime).join(name),
            )
        }
        None => crate::pipewire_lib::client::Client::connect_default(),
    };
    match connect {
        Ok(_) => 0,
        Err(_) => {
            eprintln!(
                "can't connect: {}",
                crate::tools::common::connect_failure_msg()
            );
            u8::MAX as i32
        }
    }
}

fn print_help(argv0: &str) {
    println!("{argv0} [options] [application]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -r, --remote                          Remote daemon name");
    println!("  -P, --properties                      Context properties");
    println!();
    println!("Default Context properties:");
    println!("{{");
    println!("  \"pipewire.sec.engine\": \"org.flatpak\",");
    println!("  \"pipewire.access\": \"restricted\"");
    println!("}}");
}
