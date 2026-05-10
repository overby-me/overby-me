// pw-config: PipeWire config manager.

use crate::pipewire_lib::conf;
use crate::pipewire_lib::properties::{Properties, SerializeFlags};
use crate::tools::common::{expand_short_clusters, print_version};

const DEFAULT_NAME: &str = "pipewire.conf";
const DEFAULT_PREFIX: &str = "";

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("pw-config");
    let args = expand_short_clusters(raw_args, &['h', 'V', 'r', 'L', 'N', 'C']);

    let mut opt_name = DEFAULT_NAME.to_string();
    let mut opt_prefix: String = DEFAULT_PREFIX.into();
    let mut newline = true;
    let mut recurse = false;
    let mut command: String = "paths".into();
    let mut command_args: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => {
                // End of options; remaining args are positional.
                for v in args.iter().skip(i + 1) {
                    if command == "paths" && !command_set(&mut command, v) {
                        command_args.push(v.clone());
                    }
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
            s if s.starts_with("--recurse=")
                || s.starts_with("--no-newline=")
                || s.starts_with("--no-colors=") =>
            {
                let name = s.split_once('=').map(|(n, _)| n).unwrap_or(s);
                eprintln!("{argv0}: option '{name}' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with("-n") && s.len() > 2 => {
                opt_name = s[2..].to_string();
            }
            s if s.starts_with("-p") && s.len() > 2 => {
                opt_prefix = s[2..].to_string();
            }
            opt @ ("-n" | "--name") => {
                i += 1;
                if let Some(v) = args.get(i) {
                    opt_name = v.clone();
                } else {
                    if opt == "--name" {
                        eprintln!("{argv0}: option '--name' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'n'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
            }
            s if s.starts_with("--name=") => {
                opt_name = s["--name=".len()..].to_string();
            }
            opt @ ("-p" | "--prefix") => {
                i += 1;
                if let Some(v) = args.get(i) {
                    opt_prefix = v.clone();
                } else {
                    if opt == "--prefix" {
                        eprintln!("{argv0}: option '--prefix' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'p'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
            }
            s if s.starts_with("--prefix=") => {
                opt_prefix = s["--prefix=".len()..].to_string();
            }
            "-L" | "--no-newline" => newline = false,
            "-r" | "--recurse" => recurse = true,
            "-N" | "--no-colors" | "-C" | "--color" => {
                // Color output is already off by default in non-tty mode.
            }
            s if s.starts_with("--color=") => {
                let val = &s["--color=".len()..];
                if !matches!(val, "" | "never" | "always" | "auto") {
                    eprintln!("Unknown color: {val}");
                    print_help(argv0);
                    return u8::MAX as i32;
                }
            }
            // Lone `-` is a positional, not an option.
            s if !s.starts_with('-') || s == "-" => {
                // First positional becomes the command (if recognized);
                // later ones become arguments to that command.
                if command == "paths" && command_set(&mut command, s) {
                    // command was set; don't push.
                } else {
                    command_args.push(s.to_string());
                }
            }
            // Long unknown: getopt prints `unrecognized option '<full>'`.
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            // Short unknown (single char): getopt prints
            // `invalid option -- 'X'`.
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            // Mixed cluster like `-bx`: getopt processes char-by-char,
            // erroring on the first unknown char.
            s if s.starts_with('-') => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            _ => {
                eprintln!("{argv0}: unrecognized option '{a}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
        }
        i += 1;
    }

    let prefix = if opt_prefix.is_empty() {
        None
    } else {
        Some(opt_prefix.as_str())
    };
    let configs = conf::discover(&opt_name, prefix);

    let mut assemble = Properties::new();

    match command.as_str() {
        "paths" => {
            for c in &configs {
                let key = if c.is_override {
                    format!("override.{}.{}.config.path", c.level, c.index)
                } else {
                    "config.path".into()
                };
                assemble.set(key, c.path.to_string_lossy().to_string());
            }
        }
        "merge" => {
            // Match C's behavior when no section argument is provided.
            // C maps -EINVAL → 234 (= 256 - 22) via main's truncated
            // negative-errno return.
            if command_args.is_empty() {
                eprintln!("merge requires a section");
                return 234;
            }
            // Section-aware merge is Phase 2; without a real merge, the
            // body is empty. Fall through to the assemble.serialize
            // call to get the proper `{ }`/`{\n}` formatting.
        }
        "list" => {
            // C dumps the resolved tree of configs; for our stub, list
            // each config path as section→object.
            for c in &configs {
                let key = if c.is_override {
                    format!("override.{}.{}.config.path", c.level, c.index)
                } else {
                    "config.path".into()
                };
                assemble.set(key, c.path.to_string_lossy().to_string());
            }
        }
        other => {
            eprintln!("{argv0}: command {other:?} not yet implemented in rust-pipewire");
            return 1;
        }
    }

    let flags = SerializeFlags {
        newline,
        recurse,
        enclose: true,
        array: false,
    };
    println!("{}", assemble.serialize(flags));
    0
}

fn command_set(slot: &mut String, candidate: &str) -> bool {
    if matches!(candidate, "paths" | "list" | "merge") {
        *slot = candidate.into();
        true
    } else {
        false
    }
}

fn print_help(argv0: &str) {
    println!("{argv0} : PipeWire config manager.");
    println!("Usage:");
    println!("  {argv0} [options] paths                  List config paths (default action)");
    println!("  {argv0} [options] list [SECTION]         List config section");
    println!("  {argv0} [options] merge SECTION          Merge a config section");
    println!();
    println!("Options:");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -n, --name                            Config Name (default 'pipewire.conf')");
    println!("  -p, --prefix                          Config Prefix (default '')");
    println!("  -L, --no-newline                      Omit newlines after values");
    println!("  -r, --recurse                         Reformat config sections recursively");
    println!("  -N, --no-colors                       disable color output");
    println!(
        "  -C, --color[=WHEN]                    whether to enable color support. WHEN is `never`, `always`, or `auto`"
    );
}
