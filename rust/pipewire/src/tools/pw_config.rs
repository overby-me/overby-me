// pw-config: PipeWire config manager.

use crate::pipewire_lib::conf;
use crate::pipewire_lib::properties::{Properties, SerializeFlags};
use crate::tools::common::{expand_short_clusters, print_version};

const DEFAULT_NAME: &str = "pipewire.conf";
const DEFAULT_PREFIX: &str = "";

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("pw-config");
    let args = expand_short_clusters(raw_args, &['h', 'V', 'r', 'L', 'N', 'n']);

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
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "--version" | "-V" => {
                print_version(argv0);
                return 0;
            }
            "-n" | "--name" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    opt_name = v.clone();
                } else {
                    print_help(argv0);
                    return u8::MAX as i32;
                }
            }
            "-p" | "--prefix" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    opt_prefix = v.clone();
                } else {
                    print_help(argv0);
                    return u8::MAX as i32;
                }
            }
            "-L" | "--no-newline" => newline = false,
            "-r" | "--recurse" => recurse = true,
            "-N" | "--no-colors" | "-C" | "--color" => {
                // Color output is already off by default in non-tty mode.
            }
            s if s.starts_with("--color=") => {
                // We don't emit color codes regardless of the value.
            }
            s if !s.starts_with('-') => {
                if command == "paths" && !command_set(&mut command, s) {
                    command_args.push(s.to_string());
                }
            }
            _ => {
                // Match getopt_long's "unrecognized option" message. The
                // C tool then falls through to show_help on stdout.
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
        other => {
            // list / merge are Phase 2 work — they need section parsing
            // across the resolved set of configs. For now print a clear
            // unimplemented note so callers can probe behavior.
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
