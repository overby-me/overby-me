mod engine;
mod parser;
mod regex_util;
mod types;
mod util;

use std::io::{self, Read};
use std::process;

use engine::Engine;
use parser::Parser;
use types::Options;

fn read_script_file(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("sed: -: {e}"))?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("sed: {path}: {e}"))
    }
}

fn parse_options() -> Options {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opts = Options {
        in_place: None,
        quiet: false,
        extended: false,
        expressions: Vec::new(),
        files: Vec::new(),
        posix: false,
        unbuffered: false,
        null_data: false,
        separate: false,
    };

    let mut i = 0;
    let mut saw_dashdash = false;

    while i < args.len() {
        if saw_dashdash {
            opts.files.push(args[i].clone());
            i += 1;
            continue;
        }

        match args[i].as_str() {
            "--" => {
                saw_dashdash = true;
                i += 1;
            }
            "--version" => {
                println!("sed (rust-sed) {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "--help" => {
                println!("Usage: sed [OPTION]... {{script}} [input-file]...");
                println!("  -n, --quiet, --silent    suppress automatic printing");
                println!("  -e script                add commands");
                println!("  -f file                  add commands from file");
                println!("  -i[SUFFIX]               edit files in place");
                println!("  -E, -r, --regexp-extended use extended regexes");
                println!("  -s, --separate           treat files as separate");
                println!("  -u, --unbuffered         unbuffered I/O");
                println!("  -z, --null-data          NUL-separated lines");
                println!("  --posix                  disable extensions");
                println!("  --version                print version");
                process::exit(0);
            }
            "-n" | "--quiet" | "--silent" => {
                opts.quiet = true;
                i += 1;
            }
            "-E" | "-r" | "--regexp-extended" => {
                opts.extended = true;
                i += 1;
            }
            "-e" => {
                i += 1;
                if i < args.len() {
                    opts.expressions.push(args[i].clone());
                }
                i += 1;
            }
            "-f" => {
                i += 1;
                if i < args.len() {
                    match read_script_file(&args[i]) {
                        Ok(content) => opts.expressions.push(content),
                        Err(e) => {
                            eprintln!("{e}");
                            process::exit(2);
                        }
                    }
                }
                i += 1;
            }
            "-i" => {
                opts.in_place = Some(String::new());
                i += 1;
            }
            "-s" | "--separate" => {
                opts.separate = true;
                i += 1;
            }
            "-u" | "--unbuffered" => {
                opts.unbuffered = true;
                i += 1;
            }
            "-z" | "--null-data" => {
                opts.null_data = true;
                i += 1;
            }
            "--posix" => {
                opts.posix = true;
                i += 1;
            }
            arg if arg.starts_with("-i") => {
                opts.in_place = Some(arg[2..].to_string());
                i += 1;
            }
            arg if arg.starts_with("-e") => {
                opts.expressions.push(arg[2..].to_string());
                i += 1;
            }
            arg if arg.starts_with("-f") => {
                let file = &arg[2..];
                match read_script_file(file) {
                    Ok(content) => opts.expressions.push(content),
                    Err(e) => {
                        eprintln!("{e}");
                        process::exit(2);
                    }
                }
                i += 1;
            }
            // Combined short flags like -ne, -nE
            arg if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 1 => {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'n' => opts.quiet = true,
                        'E' | 'r' => opts.extended = true,
                        'u' => opts.unbuffered = true,
                        'z' => opts.null_data = true,
                        's' => opts.separate = true,
                        'e' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            if !rest.is_empty() {
                                opts.expressions.push(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    opts.expressions.push(args[i].clone());
                                }
                            }
                            j = chars.len();
                            continue;
                        }
                        'f' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let file = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].clone()
                                } else {
                                    String::new()
                                }
                            };
                            if !file.is_empty() {
                                match read_script_file(&file) {
                                    Ok(content) => opts.expressions.push(content),
                                    Err(e) => {
                                        eprintln!("{e}");
                                        process::exit(2);
                                    }
                                }
                            }
                            j = chars.len();
                            continue;
                        }
                        'i' => {
                            let suffix: String = chars[j + 1..].iter().collect();
                            opts.in_place = Some(suffix);
                            j = chars.len();
                            continue;
                        }
                        _ => {
                            eprintln!("sed: invalid option -- '{}'", chars[j]);
                            process::exit(2);
                        }
                    }
                    j += 1;
                }
                i += 1;
            }
            arg if arg.starts_with('-') && arg.len() > 2 => {
                eprintln!("sed: unrecognized option '{arg}'");
                process::exit(2);
            }
            _ => {
                if opts.expressions.is_empty() && opts.in_place.is_none() {
                    opts.expressions.push(args[i].clone());
                } else {
                    opts.files.push(args[i].clone());
                }
                i += 1;
            }
        }
    }

    if opts.expressions.is_empty() {
        eprintln!("sed: no script command has been given");
        process::exit(2);
    }

    opts
}

fn main() {
    let opts = parse_options();

    let mut commands = Vec::new();
    let mut hash_n_quiet = false;
    for (idx, expr) in opts.expressions.iter().enumerate() {
        let mut parser = Parser::new(expr, opts.extended);
        match parser.parse_all(idx == 0) {
            Ok(cmds) => {
                if idx == 0 && parser.hash_n_quiet {
                    hash_n_quiet = true;
                }
                commands.extend(cmds);
            }
            Err(e) => {
                eprintln!("sed: {e}");
                process::exit(2);
            }
        }
    }

    let quiet = opts.quiet || hash_n_quiet;

    let stdout = io::stdout();
    let mut out = stdout.lock();

    if opts.in_place.is_some() {
        let suffix = opts.in_place.as_deref().unwrap_or("");
        if opts.files.is_empty() {
            eprintln!("sed: no input files for in-place editing");
            process::exit(2);
        }
        for file in &opts.files {
            let content = match std::fs::read_to_string(file) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("sed: {file}: {e}");
                    continue;
                }
            };

            if !suffix.is_empty() {
                let backup = format!("{file}{suffix}");
                if let Err(e) = std::fs::copy(file, &backup) {
                    eprintln!("sed: cannot create backup {backup}: {e}");
                    continue;
                }
            }

            let reader = io::BufReader::new(content.as_bytes());
            let mut output = Vec::new();
            let mut engine = Engine::new(commands.clone(), quiet);
            let code = engine.run(reader, &mut output).unwrap_or_else(|e| {
                eprintln!("sed: {file}: {e}");
                1
            });

            if let Err(e) = std::fs::write(file, &output) {
                eprintln!("sed: {file}: {e}");
            }

            if code != 0 {
                process::exit(code);
            }
        }
    } else if opts.files.is_empty() || (opts.files.len() == 1 && opts.files[0] == "-") {
        let stdin = io::stdin();
        let reader = stdin.lock();
        let mut engine = Engine::new(commands, quiet);
        let code = engine.run(reader, &mut out).unwrap_or_else(|e| {
            eprintln!("sed: {e}");
            1
        });
        process::exit(code);
    } else {
        let mut engine = Engine::new(commands, quiet);
        for file in &opts.files {
            let content = if file == "-" {
                let mut buf = String::new();
                io::stdin().read_to_string(&mut buf).unwrap_or_default();
                buf
            } else {
                match std::fs::read_to_string(file) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("sed: {file}: {e}");
                        continue;
                    }
                }
            };

            engine.current_filename = Some(file.clone());
            let reader = io::BufReader::new(content.as_bytes());
            if let Err(e) = engine.run(reader, &mut out) {
                eprintln!("sed: {file}: {e}");
            }
        }
    }
}
