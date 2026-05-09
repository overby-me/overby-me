// pw-reserve: D-Bus device reservation. Phase 7+ implementation deferred;
// for now we just emit upstream's exact help/version output.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    // Upstream pw-reserve passes argv[0] verbatim — no basename — so the
    // help text shows the full /nix/store/... path when invoked that way.
    // Tests normalize that path to `TOOL` before diffing.
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-reserve");

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
            s if s.starts_with('-')
                && !matches!(
                    s,
                    "-n" | "--name"
                        | "-a"
                        | "--appname"
                        | "-p"
                        | "--priority"
                        | "-m"
                        | "--monitor"
                        | "-r"
                        | "--release"
                ) =>
            {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            _ => {}
        }
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
