// spa-json-dump: parse SPA-relaxed JSON and emit pretty-printed JSON.
//
// Aims for byte-compatibility with `spa/tools/spa-json-dump.c`.

use std::fs::File;
use std::io::{self, Read};

use crate::spa::utils::json::{DumpOptions, dump, parse};

const DEFAULT_INDENT: usize = 2;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("spa-json-dump");

    let mut indent = DEFAULT_INDENT;
    let mut simple = false;
    let mut filename: String = "-".into();

    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            s if s.starts_with("--help=") => {
                eprintln!("{argv0}: option '--help' doesn't allow an argument");
                print_help_to(argv0, true);
                return 0;
            }
            // spa-json-dump's getopt_long doesn't include --version or -V.
            // Long unknowns trip the standard `unrecognized option` path;
            // short unknowns trip `invalid option -- 'X'`.
            "-s" | "--spa" => simple = true,
            "-i" | "--indent" => {
                i += 1;
                let arg = match args.get(i) {
                    Some(a) => a,
                    None => {
                        print_help_to(argv0, true);
                        return u8::MAX as i32; // C tool returns -1 → 255
                    }
                };
                indent = arg.parse::<usize>().unwrap_or(DEFAULT_INDENT);
            }
            // Accept `-i<N>` joined form to mirror getopt_long behavior.
            s if s.starts_with("-i") && s.len() > 2 => {
                indent = s[2..].parse::<usize>().unwrap_or(DEFAULT_INDENT);
            }
            s if s.starts_with("--indent=") => {
                indent = s[9..].parse::<usize>().unwrap_or(DEFAULT_INDENT);
            }
            s if !s.starts_with('-') => {
                filename = s.to_string();
                // Anything after the filename is currently ignored.
                break;
            }
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help_to(argv0, true);
                return 1;
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help_to(argv0, true);
                return 1;
            }
            _ => {
                print_help_to(argv0, true);
                return u8::MAX as i32;
            }
        }
        i += 1;
    }

    let input = match read_input(&filename) {
        Ok(s) => s,
        Err(e) => {
            // Match the C tool's error wording closely. Strip Rust's
            // trailing `(os error N)` so the message ends with just the
            // strerror text like upstream's `%m`.
            let msg = e
                .to_string()
                .rsplit_once(" (os error ")
                .map(|(m, _)| m.to_string())
                .unwrap_or_else(|| e.to_string());
            eprintln!("error opening file '{filename}': {msg}");
            return 1;
        }
    };

    let opts = DumpOptions { indent, simple };

    let value = match parse(&input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "syntax error in file '{filename}' (line {}, col {}): {}",
                e.line, e.column, e.message
            );
            return 1;
        }
    };

    println!("{}", dump(&value, &opts));
    0
}

fn read_input(filename: &str) -> io::Result<String> {
    if filename == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        let mut f = File::open(filename)?;
        let mut buf = String::new();
        f.read_to_string(&mut buf)?;
        Ok(buf)
    }
}

fn print_help(argv0: &str) {
    print_help_to(argv0, false);
}

fn print_help_to(argv0: &str, _is_error: bool) {
    // Matches `src/tools/spa-json-dump.c::show_usage` byte-for-byte.
    println!("{argv0} [options] [spa-json-file]");
    println!("  -h, --help                            Show this help");
    println!();
    println!("  -i  --indent                          set indent (default {DEFAULT_INDENT})");
    println!("  -s  --spa                             use simplified SPA JSON");
    println!();
}
