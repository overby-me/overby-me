// pw-top: real-time PipeWire performance viewer.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-top");
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "-V" | "--version" => {
                print_version(argv0);
                return 0;
            }
            "-b" | "--batch-mode" => {}
            "-n" | "--iterations" => {
                if i + 1 >= args.len() {
                    eprintln!("{argv0}: option requires an argument -- 'n'");
                    print_help(argv0);
                    return 0;
                }
                i += 2;
                continue;
            }
            "-r" | "--remote" => {
                if i + 1 >= args.len() {
                    eprintln!("{argv0}: option requires an argument -- 'r'");
                    print_help(argv0);
                    return 0;
                }
                i += 2;
                continue;
            }
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with('-') => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            _ => {}
        }
        i += 1;
    }
    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
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
