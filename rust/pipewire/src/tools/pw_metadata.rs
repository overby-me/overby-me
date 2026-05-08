// pw-metadata: read/write PipeWire metadata.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-metadata");
    match args.get(1).map(String::as_str) {
        Some("-h") | Some("--help") => {
            print_help(argv0);
            0
        }
        Some("--version") => {
            print_version(argv0);
            0
        }
        _ => {
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
    }
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
