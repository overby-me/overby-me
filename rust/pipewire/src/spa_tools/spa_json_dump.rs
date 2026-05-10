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
            "--" => {
                // getopt_long: end-of-options marker. Next arg (if any)
                // is the positional filename; default stays "-" if none.
                if let Some(s) = args.get(i + 1) {
                    filename = s.clone();
                }
                break;
            }
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
            opt @ ("-i" | "--indent") => {
                i += 1;
                let arg = match args.get(i) {
                    Some(a) => a,
                    None => {
                        if opt == "--indent" {
                            eprintln!(
                                "{argv0}: option '--indent' requires an argument"
                            );
                        } else {
                            eprintln!(
                                "{argv0}: option requires an argument -- 'i'"
                            );
                        }
                        print_help_to(argv0, true);
                        return u8::MAX as i32;
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
            s if !s.starts_with('-') || s == "-" => {
                // `-` is the conventional stdin marker; C's spa-json-dump
                // treats it identically to no filename (default reads
                // stdin).
                filename = s.to_string();
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
            // Mixed cluster like `-bx`: getopt processes char-by-char,
            // so a cluster starting with `-h` short-circuits to help
            // before scanning the remaining chars.
            s if s.starts_with('-') && !s.starts_with("--") => {
                let ch = s.chars().nth(1).unwrap_or('?');
                if ch == 'h' {
                    print_help(argv0);
                    return 0;
                }
                // Iterate over each char in the cluster. `-s` is no-arg,
                // `-h` short-circuits to help. Anything else errors.
                let mut chars = s[1..].chars();
                while let Some(c) = chars.next() {
                    if c == 'h' {
                        print_help(argv0);
                        return 0;
                    } else if c == 's' {
                        simple = true;
                    } else if c == 'i' {
                        // -i takes the rest of the cluster as the value
                        // (or the next argv if cluster ends).
                        let remaining: String = chars.collect();
                        if !remaining.is_empty() {
                            indent = remaining.parse::<usize>().unwrap_or(DEFAULT_INDENT);
                        } else if let Some(v) = args.get(i + 1) {
                            indent = v.parse::<usize>().unwrap_or(DEFAULT_INDENT);
                            i += 1;
                        } else {
                            eprintln!("{argv0}: option requires an argument -- 'i'");
                            print_help_to(argv0, true);
                            return u8::MAX as i32;
                        }
                        break;
                    } else {
                        eprintln!("{argv0}: invalid option -- '{c}'");
                        print_help_to(argv0, true);
                        return 1;
                    }
                }
            }
            _ => {
                print_help_to(argv0, true);
                return u8::MAX as i32;
            }
        }
        i += 1;
    }

    // Mirror C's spa-json-dump.c flow:
    //   - filename == "" or "-": "not a valid file '<name>': Success"
    //     (errno is 0 since the C path doesn't call any libc that sets it)
    //   - open(...) fails: "error opening file '<name>': <strerror>"
    //   - mmap(fd, st_size) fails: "error mmapping file '<name>': <strerror>"
    //     - empty regular file (size=0):     EINVAL → "Invalid argument"
    //     - directory:                       ENODEV → "No such device"
    if filename.is_empty() {
        eprintln!("not a valid file '{filename}': Success");
        return 1;
    }
    // Stdin path: C reads stdin into a buffer, then if the buffer is
    // empty (e.g. </dev/null) reports "not a valid file '-': Success".
    // Non-empty stdin is parsed normally.
    if filename == "-" {
        let mut buf = String::new();
        let _ = io::stdin().read_to_string(&mut buf);
        // C reports "not a valid file" when the buffer contains nothing
        // parseable — empty / whitespace-only / comment-only input.
        if has_no_content(&buf) {
            eprintln!("not a valid file '{filename}': Success");
            return 1;
        }
        return parse_and_dump(&filename, &buf, indent, simple);
    }
    // Probe stat() size up-front: C's mmap(size) fails with EINVAL when
    // st_size == 0. Files in /proc and /sys report size 0 even though
    // read() returns data, so we have to check stat directly rather than
    // relying on the read result.
    if let Ok(meta) = std::fs::metadata(&filename) {
        if meta.is_file() && meta.len() == 0 {
            eprintln!("error mmapping file '{filename}': Invalid argument");
            return 1;
        }
    }
    let input = match read_input(&filename) {
        Ok(s) if s.is_empty() => {
            // Mirror mmap(size=0) → EINVAL.
            eprintln!("error mmapping file '{filename}': Invalid argument");
            return 1;
        }
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
            // Directories: open succeeds in C, mmap fails with ENODEV
            // (Rust's File::open returns "Is a directory" upfront).
            if msg == "Is a directory" {
                eprintln!("error mmapping file '{filename}': No such device");
            } else {
                eprintln!("error opening file '{filename}': {msg}");
            }
            return 1;
        }
    };

    parse_and_dump(&filename, &input, indent, simple)
}

fn has_no_content(input: &str) -> bool {
    // Strip whitespace and `#`-style line comments; if nothing
    // substantial remains, C considers the input to have no content.
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '#' {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
        } else {
            return false;
        }
    }
    true
}

fn parse_and_dump(filename: &str, input: &str, indent: usize, simple: bool) -> i32 {
    let opts = DumpOptions { indent, simple };

    let value = match parse(input) {
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
