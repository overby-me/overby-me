// pw-mididump: dump MIDI events from a Standard MIDI File.
//
// When given a filename, reads the SMF and emits the same per-event lines
// that the C tool's `midi_event_dump` does. The "live filter" mode (no
// filename) requires a daemon — Phase 7 territory.

use crate::spa::control::midi;
use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-mididump");

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
            "-r" | "--remote" => {
                i += 1;
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
            s if !s.starts_with('-') => {
                filename = Some(s.to_string());
                break;
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
            // Live-filter mode would need a daemon — Phase 7. Print a
            // clear message and exit non-zero so callers know.
            eprintln!(
                "{argv0}: live MIDI capture requires a running PipeWire daemon (not yet implemented in rust-pipewire)"
            );
            return 1;
        }
    };

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
            // without Rust's `(os error N)` parenthetical.
            let msg = e
                .to_string()
                .rsplit_once(" (os error ")
                .map(|(m, _)| m.to_string())
                .unwrap_or_else(|| e.to_string());
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
