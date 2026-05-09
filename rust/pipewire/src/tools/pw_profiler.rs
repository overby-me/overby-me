// pw-profiler: collect profiler events.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-profiler");
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
    println!("  -r, --remote                          Remote daemon name");
    println!(
        "  -o, --output                          Profiler output name (default \"profiler.log\")"
    );
    println!("  -J, --json                            Dump raw data as JSON");
    println!("  -n, --iterations                      Collect this many samples");
}
