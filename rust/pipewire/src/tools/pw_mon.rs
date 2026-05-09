// pw-mon: PipeWire registry monitor.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-mon");
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
            "-r" | "--remote" | "-N" | "--no-colors" | "-o" | "--hide-props"
            | "-a" | "--hide-params" | "-p" | "--print-separator" => {}
            s if s.starts_with("--color") || s.starts_with("-C") => {}
            s if s.starts_with('-') => {
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
    println!("  -r, --remote                          Remote daemon name");
    println!("  -N, --no-colors                       disable color output");
    println!(
        "  -C, --color[=WHEN]                    whether to enable color support. WHEN is `never`, `always`, or `auto`"
    );
    println!("  -o, --hide-props                      hide node properties");
    println!("  -a, --hide-params                     hide node parameters");
    println!(
        "  -p, --print-separator                 print empty line after every event to help streaming parser"
    );
}
