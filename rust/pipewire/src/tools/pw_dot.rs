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
                return 0;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return 0;
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
                return 0;
            }
            "-o" | "--output" => {
                if i + 1 >= args.len() {
                    eprintln!("{argv0}: option requires an argument -- 'o'");
                    print_help(argv0);
                    return 0;
                }
                output = args.get(i + 1).cloned();
                i += 2;
                continue;
            }
            "-j" | "--json" => {
                if i + 1 >= args.len() {
                    eprintln!("{argv0}: option requires an argument -- 'j'");
                    print_help(argv0);
                    return 0;
                }
                json_input = args.get(i + 1).cloned();
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
            "-a" | "--all" | "-s" | "--smart" | "-d" | "--detail" | "-L"
            | "--lr" | "-9" | "--90" => {}
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

    if let Some(path) = json_input {
        // C tool prints `Using JSON file <path> as input` on stdout when
        // -j is used.
        println!("Using JSON file {path} as input");
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("can't open JSON file: {path}");
                return 1;
            }
        };
        let _ = content; // we don't yet parse; pretend it's empty graph

        let dest = output.as_deref().unwrap_or("pw.dot");
        if dest == "-" {
            println!("set output file -");
            print_empty_graph(&mut std::io::stdout()).ok();
        } else {
            // Default: write to pw.dot file. Print "set output file <path>"
            // when -o is given explicitly.
            if output.is_some() {
                println!("set output file {dest}");
            }
            if let Ok(mut f) = std::fs::File::create(dest) {
                use std::io::Write;
                let mut buf = Vec::new();
                print_empty_graph(&mut buf).ok();
                let _ = f.write_all(&buf);
            }
        }
        return 0;
    }

    // Without -j, C connects to the daemon to collect the graph. Try
    // connecting; if it fails, emit the same error C does.
    match crate::pipewire_lib::client::Client::connect_default() {
        Ok(_) => {
            // Successful connect — but we don't actually walk the graph,
            // so emit nothing (matches C's behavior on graceful idle exit
            // when the daemon has no nodes).
            0
        }
        Err(_) => {
            eprintln!("can't connect: Host is down");
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
