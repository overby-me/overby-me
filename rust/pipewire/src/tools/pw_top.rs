// pw-top: real-time PipeWire performance viewer.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-top");
    match args.get(1).map(String::as_str) {
        Some("-h") | Some("--help") => {
            print_help(argv0);
            0
        }
        Some("-V") | Some("--version") => {
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
    // pw-top has its own format with a "Usage:" header and tab-aligned
    // batch-mode option. Bytes match `src/tools/pw-top.c::show_help`.
    println!("Usage:");
    println!("{argv0} [options]");
    println!();
    println!("Options:");
    println!("  -b, --batch-mode\t\t         run in non-interactive batch mode");
    println!("  -n, --iterations = NUMBER             exit after NUMBER batch iterations");
    println!("  -r, --remote                          Remote daemon name");
    println!();
    println!("  -h, --help                            Show this help");
    println!("  -V  --version                         Show version");
}
