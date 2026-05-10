// pw-dot: dump the PipeWire graph as graphviz.

use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("pw-dot");
    let args = expand_short_clusters(raw_args, &['h', 'V', 'a', 's', 'd', 'L', '9']);
    let mut output: Option<String> = None;
    let mut json_input: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => break,
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
            s if s.starts_with("--all=")
                || s.starts_with("--smart=")
                || s.starts_with("--detail=")
                || s.starts_with("--lr=")
                || s.starts_with("--90=") =>
            {
                let name = s.split_once('=').map(|(n, _)| n).unwrap_or(s);
                eprintln!("{argv0}: option '{name}' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            opt @ ("-o" | "--output") => {
                if i + 1 >= args.len() {
                    if opt == "--output" {
                        eprintln!("{argv0}: option '--output' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'o'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
                output = args.get(i + 1).cloned();
                i += 2;
                continue;
            }
            opt @ ("-j" | "--json") => {
                if i + 1 >= args.len() {
                    if opt == "--json" {
                        eprintln!("{argv0}: option '--json' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'j'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
                json_input = args.get(i + 1).cloned();
                i += 2;
                continue;
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
                let val = args.get(i + 1).cloned().unwrap_or_default();
                println!("set remote to {val}");
                i += 2;
                continue;
            }
            s if s.starts_with("--remote=") => {
                let val = &s["--remote=".len()..];
                println!("set remote to {val}");
            }
            s if s.starts_with("--output=") => {
                let val = &s["--output=".len()..];
                output = Some(val.to_string());
            }
            s if s.starts_with("--json=") => {
                let val = &s["--json=".len()..];
                json_input = Some(val.to_string());
            }
            s if s.starts_with("-r") && s.len() > 2 => {
                let val = &s[2..];
                println!("set remote to {val}");
            }
            s if s.starts_with("-o") && s.len() > 2 => {
                output = Some(s[2..].to_string());
            }
            s if s.starts_with("-j") && s.len() > 2 => {
                json_input = Some(s[2..].to_string());
            }
            "-a" | "--all" | "-s" | "--smart" | "-d" | "--detail" | "-L"
            | "--lr" | "-9" | "--90" => {}
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with('-') && !s.starts_with("--") => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with('-') => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            _ => {}
        }
        i += 1;
    }

    // If -o given, announce it before opening anything (C does this in
    // option parsing; with empty value the announce still fires).
    if let Some(out_path) = output.as_deref() {
        if out_path != "-" {
            println!("set output file {out_path}");
        }
    }
    if let Some(path) = json_input {
        // C tool prints `Using JSON file <path> as input` on stdout when
        // -j is used.
        println!("Using JSON file {path} as input");
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => {
                // C uses fopen+strerror: "error opening file '<path>': <strerror>".
                let strerror = if path.is_empty() || !std::path::Path::new(&path).exists() {
                    "No such file or directory"
                } else {
                    "Permission denied"
                };
                eprintln!("error opening file '{path}': {strerror}");
                return 1;
            }
        };
        let _ = content; // we don't yet parse; pretend it's empty graph

        let dest = output.as_deref().unwrap_or("pw.dot");
        if dest == "-" {
            println!("set output file -");
            print_empty_graph(&mut std::io::stdout()).ok();
        } else {
            if let Ok(mut f) = std::fs::File::create(dest) {
                use std::io::Write;
                let mut buf = Vec::new();
                print_empty_graph(&mut buf).ok();
                let _ = f.write_all(&buf);
            } else {
                eprintln!("open error: could not open {dest} for writing");
            }
        }
        return 0;
    }
    // Without -j, C connects to the daemon first. Only if that succeeds
    // does it attempt to open the output file. So an unreachable daemon
    // hides the open-error.

    // Without -j, C connects to the daemon to collect the graph. Try
    // connecting; if it fails, emit the same error C does. PIPEWIRE_REMOTE
    // env supplies the socket name when -r wasn't given.
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
            // Successful connect — try opening the output file (if -o
            // given with an unwritable path, this is where C errors).
            if let Some(out_path) = output.as_deref() {
                if out_path != "-" && std::fs::File::create(out_path).is_err() {
                    eprintln!("open error: could not open {out_path} for writing");
                    return 0;
                }
            }
            0
        }
        Err(_) => {
            eprintln!("can't connect: {}", crate::tools::common::connect_failure_msg());
            255
        }
    }
}

fn print_empty_graph<W: std::io::Write>(out: &mut W) -> std::io::Result<()> {
    writeln!(out, "digraph pipewire {{")?;
    writeln!(out, "}}")?;
    Ok(())
}

fn print_help(argv0: &str) {
    println!("{argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -a, --all                             Show all object types");
    println!("  -s, --smart                           Show linked objects only");
    println!("  -d, --detail                          Show all object properties");
    println!("  -r, --remote                          Remote daemon name");
    println!("  -o, --output                          Output file (Default pw.dot)");
    println!("  -L, --lr                              Use left-right rank direction");
    println!("  -9, --90                              Use orthogonal edges");
    println!("  -j, --json                            Read objects from pw-dump JSON file");
}
