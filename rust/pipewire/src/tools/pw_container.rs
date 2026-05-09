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
            _ => {}
        }
    }

    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
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
