// pw-reserve: D-Bus device reservation. Phase 7+ implementation deferred;
// for now we just emit upstream's exact help/version output.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    // Upstream pw-reserve passes argv[0] verbatim — no basename — so the
    // help text shows the full /nix/store/... path when invoked that way.
    // Tests normalize that path to `TOOL` before diffing.
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-reserve");

    let mut name: Option<String> = None;
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
                return 0;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return 0;
            }
            "-n" | "--name" => {
                if let Some(v) = args.get(i + 1) {
                    name = Some(v.clone());
                    i += 2;
                    continue;
                }
            }
            s if s.starts_with("--name=") => {
                name = Some(s["--name=".len()..].to_string());
            }
            "-a" | "--appname" | "-p" | "--priority" => {
                i += 2;
                continue;
            }
            "-m" | "--monitor" | "-r" | "--release" => {}
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with('-') => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return 0;
            }
            _ => {}
        }
        i += 1;
    }

    // C's pw-reserve checks for a valid name (Audio0, Midi0, Video0, ..)
    // before doing anything else.
    if name.is_none() {
        eprintln!("valid name must be given");
        return 0;
    }
    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
}

fn print_help(argv0: &str) {
    println!("{argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -n, --name                            Name to reserve (Audio0, Midi0, Video0, ..)");
    println!("  -a, --appname                         Application Name (default pw-reserve)");
    println!("  -p, --priority                        Priority (default 0)");
    println!("  -m, --monitor                         Monitor only, don't try to acquire");
    println!("  -r, --release                         Request release when busy");
}
