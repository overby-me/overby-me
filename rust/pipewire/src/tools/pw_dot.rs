// pw-dot: dump the PipeWire graph as graphviz.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-dot");
    match args.get(1).map(String::as_str) {
        Some("-h") | Some("--help") => {
            print_help(argv0);
            0
        }
        Some("--version") | Some("-V") => {
            print_version(argv0);
            0
        }
        Some(s) if s.starts_with('-') => {
            eprintln!("{argv0}: unrecognized option '{s}'");
            print_help(argv0);
            0
        }
        _ => {
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
    }
}

fn print_help(argv0: &str) {
    println!("{argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -a, --all                             Show all object types");
    println!("  -s, --smart                           Show linked objects only");
    println!("  -d, --detail                          Show all object properties");
    println!("  -r, --remote                          Remote daemon name");
    println!("  -o, --output                          Output file (Default pw.dot)");
    println!("  -L, --lr                              Use left-right rank direction");
    println!("  -9, --90                              Use orthogonal edges");
    println!("  -j, --json                            Read objects from pw-dump JSON file");
}
