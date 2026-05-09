// pw-profiler: collect profiler events.

use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("pw-profiler");
    let args = expand_short_clusters(raw_args, &['h', 'V', 'J']);
    match args.get(1).map(String::as_str) {
        Some("-h") | Some("--help") => {
            print_help(argv0);
            0
        }
        Some("--version") | Some("-V") => {
            print_version(argv0);
            0
        }
        Some(s) if s.starts_with("--") => {
            eprintln!("{argv0}: unrecognized option '{s}'");
            print_help(argv0);
            0
        }
        Some(s) if s.starts_with('-') && s.len() == 2 => {
            let ch = s.chars().nth(1).unwrap_or('?');
            eprintln!("{argv0}: invalid option -- '{ch}'");
            print_help(argv0);
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
