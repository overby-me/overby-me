// pw-mididump: dump MIDI events from a Standard MIDI File.
//
// When given a filename, reads the SMF and emits the same per-event lines
// that the C tool's `midi_event_dump` does. The "live filter" mode (no
// filename) requires a daemon — Phase 7 territory.

use crate::spa::control::midi;
use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args
        .first()
        .map(String::as_str)
        .unwrap_or("pw-mididump");
    let args = expand_short_clusters(raw_args, &['h', 'V']);

    let mut filename: Option<String> = None;
    let mut force_ump = false;
    let mut i = 1;

    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "--version" | "-V" => {
                print_version(argv0);
                return 0;
            }
            s if s.starts_with("--help=") => {
                eprintln!("{argv0}: option '--help' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            "--" => {
                // getopt_long: end-of-options marker. Remaining args are
                // positional filenames; only the first is used.
                for s in args.iter().skip(i + 1) {
                    if filename.is_none() {
                        filename = Some(s.to_string());
                    }
                }
                break;
            }
            opt @ ("-r" | "--remote") => {
                if i + 1 >= args.len() {
                    if opt == "--remote" {
                        eprintln!("{argv0}: option '--remote' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'r'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
                i += 1;
            }
            s if s.starts_with("--remote=") => {
                // Inline value (including empty `--remote=`); accepted.
            }
            s if s.starts_with("-r") && s.len() > 2 => {
                // `-r<value>` (no space) — accepted, value is consumed.
            }
            "-M" | "--force-midi" => {
                i += 1;
                let val = match args.get(i) {
                    Some(v) => v.as_str(),
                    None => {
                        // Match C getopt's `option requires an argument`.
                        // -M is the short form; long would be --force-midi.
                        let flag_short = a == "-M";
                        if flag_short {
                            eprintln!("{argv0}: option requires an argument -- 'M'");
                        } else {
                            eprintln!("{argv0}: option '--force-midi' requires an argument");
                        }
                        print_help(argv0);
                        return u8::MAX as i32;
                    }
                };
                match val {
                    "midi" => force_ump = false,
                    "ump" => force_ump = true,
                    _ => {
                        eprintln!("error: bad force-midi {val}");
                        print_help(argv0);
                        return 0;
                    }
                }
            }
            s if s.starts_with("-M") && s.len() > 2 => {
                // `-M<value>` attached form (getopt doesn't strip a
                // leading `=`, so `-M=ump` validates the literal "=ump").
                let val = &s[2..];
                match val {
                    "midi" => force_ump = false,
                    "ump" => force_ump = true,
                    _ => {
                        eprintln!("error: bad force-midi {val}");
                        print_help(argv0);
                        return 0;
                    }
                }
            }
            s if s.starts_with("--force-midi=") => {
                let val = &s["--force-midi=".len()..];
                match val {
                    "midi" => force_ump = false,
                    "ump" => force_ump = true,
                    _ => {
                        eprintln!("error: bad force-midi {val}");
                        print_help(argv0);
                        return 0;
                    }
                }
            }
            s if !s.starts_with('-') || s == "-" => {
                // `-` is the conventional stdin marker, treated as a
                // filename by the C tool (which then fopen's it and
                // fails with "Invalid argument" because "-" isn't a real
                // path on disk). C's getopt permutes argv so options
                // after positionals are still processed; we don't break.
                // Only the FIRST positional is the filename — later ones
                // are silently ignored.
                if filename.is_none() {
                    filename = Some(s.to_string());
                }
            }
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            // Short options not handled above (single char or cluster):
            // getopt prints `invalid option -- 'X'`.
            s if s.starts_with('-') => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
        }
        i += 1;
    }
    let _ = force_ump; // ump support is filter-mode only for now

    let filename = match filename {
        Some(f) => f,
        None => {
            // Live-filter mode connects to the daemon. Without one,
            // C's pw_context_connect fails and pw-mididump prints
            // "can't connect" then a segfault on null deref. We just
            // emit the error. PIPEWIRE_REMOTE supplies the socket name.
            let env_remote = std::env::var("PIPEWIRE_REMOTE").ok();
            let connect = if let Some(name) = &env_remote {
                let runtime = std::env::var("PIPEWIRE_RUNTIME_DIR")
                    .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
                    .unwrap_or_else(|_| "/tmp".to_string());
                let path = std::path::PathBuf::from(runtime).join(name);
                crate::pipewire_lib::client::Client::connect_path(&path)
            } else {
                crate::pipewire_lib::client::Client::connect_default()
            };
            match connect {
                Ok(_) => {
                    // Daemon up; we don't yet stream events. Exit 0
                    // silently to avoid spurious hangs on host machines.
                    return 0;
                }
                Err(_) => {
                    eprintln!("can't connect");
                    return u8::MAX as i32;
                }
            }
        }
    };

    // Note: filename == "-" reads from stdin (handled by midi::read_file).
    // Empty stdin yields a parse error → "Invalid argument" via the
    // generic Err path below.

    match midi::read_file(&filename) {
        Ok((info, events)) => {
            // Mirror dump_file() in src/tools/pw-mididump.c:
            //   "opened %s format:%u ntracks:%u division:%u\n"
            println!(
                "opened {filename} format:{} ntracks:{} division:{}",
                info.format, info.ntracks, info.division
            );
            for ev in &events {
                println!("{}", midi::format_event(ev));
            }
            0
        }
        Err(e) => {
            // Match the C tool's `%m` output, which is just strerror(errno)
            // without Rust's `(os error N)` parenthetical. C's fopen
            // succeeds on directories then read_mthd returns -EINVAL
            // because fread can't read 14 header bytes; mirror that as
            // "Invalid argument" instead of Rust's "Is a directory".
            let msg = e
                .to_string()
                .rsplit_once(" (os error ")
                .map(|(m, _)| m.to_string())
                .unwrap_or_else(|| e.to_string());
            let msg = if msg == "Is a directory" {
                "Invalid argument".to_string()
            } else {
                msg
            };
            eprintln!("error opening {filename}: {msg}");
            // C tool returns -1 from main, which truncates to 255.
            255
        }
    }
}

fn print_help(argv0: &str) {
    println!("{argv0} [options] [FILE]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -r, --remote                          Remote daemon name");
    println!(
        "  -M, --force-midi                      Force midi format, one of \"midi\" or \"ump\",(default midi)"
    );
}
