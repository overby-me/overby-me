// spa-json-dump: parse SPA-relaxed JSON and emit pretty-printed JSON.
// Phase 0: only --help. Real parser comes in Phase 1.

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("spa-json-dump");

    match args.get(1).map(String::as_str) {
        Some("--version") => {
            let v = crate::pipewire_lib::version::PIPEWIRE_API_VERSION;
            println!("{argv0}");
            println!("Compiled with libpipewire {v}");
            println!("Linked with libpipewire {v}");
            0
        }
        Some("-h") | Some("--help") | None => {
            print_help(argv0);
            0
        }
        // Real flag parsing (-i/-s) and file handling come in Phase 1.
        Some(_) => {
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
    }
}

fn print_help(argv0: &str) {
    // Matches `src/tools/spa-json-dump.c::show_help` byte-for-byte
    // (after store-path normalization).
    println!("{argv0} [options] [spa-json-file]");
    println!("  -h, --help                            Show this help");
    println!();
    println!("  -i  --indent                          set indent (default 2)");
    println!("  -s  --spa                             use simplified SPA JSON");
    println!();
}
