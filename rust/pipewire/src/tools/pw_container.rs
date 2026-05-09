// pw-container: namespace/container helper. Real implementation is Phase
// 9-ish (it spawns child processes inside a sandboxed namespace). Help
// text matches upstream byte-for-byte.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    // pw-container passes argv[0] verbatim to its help printer.
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-container");

    for a in args.iter().skip(1) {
        match a.as_str() {
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
            s if s.starts_with('-') && !matches!(s, "-r" | "--remote" | "-P" | "--properties") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            _ => {}
        }
    }

    // C connects to the daemon to fork an application inside a sandboxed
    // context. Try connecting; emit the same "can't connect" error.
    match crate::pipewire_lib::client::Client::connect_default() {
        Ok(_) => 0,
        Err(_) => {
            eprintln!("can't connect: {}", crate::tools::common::connect_failure_msg());
            0
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
