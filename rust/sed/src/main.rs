mod engine;
mod parser;
mod regex_util;
mod types;
mod util;

#[allow(unused_imports)]
use std::io::{self, Read, Write};
use std::process;

use engine::Engine;
use parser::Parser;
use types::{Options, ScriptEntry, ScriptSource};

/// Format an IO error like GNU sed (strip Rust's "(os error N)" suffix)
fn fmt_io_err(e: &io::Error) -> String {
    let msg = e.to_string();
    if let Some(code) = e.raw_os_error() {
        msg.strip_suffix(&format!(" (os error {code})"))
            .unwrap_or(&msg)
            .to_string()
    } else {
        msg
    }
}

fn print_usage(out: &mut dyn io::Write, full: bool) {
    let _ = writeln!(out, "Usage: sed [OPTION]... {{script}} [input-file]...");
    let _ = writeln!(out, "  -n, --quiet, --silent    suppress automatic printing");
    let _ = writeln!(out, "  -e script                add commands");
    let _ = writeln!(out, "  -f file                  add commands from file");
    let _ = writeln!(out, "  -i[SUFFIX]               edit files in place");
    let _ = writeln!(out, "  -E, -r, --regexp-extended use extended regexes");
    let _ = writeln!(out, "  -s, --separate           treat files as separate");
    let _ = writeln!(out, "  -u, --unbuffered         unbuffered I/O");
    let _ = writeln!(out, "  -z, --null-data          NUL-separated lines");
    let _ = writeln!(out, "  --posix                  disable extensions");
    let _ = writeln!(out, "  --version                print version");
    let _ = writeln!(out);
    if full {
        let _ = writeln!(out, "E-mail bug reports to: bug-sed@gnu.org");
    }
}

fn read_script_file(path: &str) -> Result<String, String> {
    if path == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("sed: -: {}", fmt_io_err(&e)))?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("sed: {path}: {}", fmt_io_err(&e)))
    }
}

fn parse_options() -> Options {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut expr_count = 0usize; // tracks -e expression numbering
    let mut opts = Options {
        in_place: None,
        quiet: false,
        extended: false,
        scripts: Vec::new(),
        files: Vec::new(),
        posix: false,
        unbuffered: false,
        null_data: false,
        separate: false,
        sandbox: false,
        line_length: 70,
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
                print_usage(&mut io::stdout(), true);
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
                    expr_count += 1;
                    opts.scripts.push(ScriptEntry {
                        source: ScriptSource::Expression(expr_count),
                        content: args[i].clone(),
                    });
                }
                i += 1;
            }
            "-f" => {
                i += 1;
                if i < args.len() {
                    match read_script_file(&args[i]) {
                        Ok(content) => opts.scripts.push(ScriptEntry {
                            source: ScriptSource::File(args[i].clone()),
                            content,
                        }),
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
            "--sandbox" => {
                opts.sandbox = true;
                i += 1;
            }
            "--follow-symlinks" => {
                // Accepted but ignored (Linux-only GNU extension)
                i += 1;
            }
            "-l" => {
                i += 1;
                if i < args.len() {
                    opts.line_length = args[i].parse().unwrap_or(70);
                }
                i += 1;
            }
            arg if arg.starts_with("-i") => {
                opts.in_place = Some(arg[2..].to_string());
                i += 1;
            }
            arg if arg.starts_with("-e") => {
                expr_count += 1;
                opts.scripts.push(ScriptEntry {
                    source: ScriptSource::Expression(expr_count),
                    content: arg[2..].to_string(),
                });
                i += 1;
            }
            arg if arg.starts_with("-f") => {
                let file = &arg[2..];
                match read_script_file(file) {
                    Ok(content) => opts.scripts.push(ScriptEntry {
                        source: ScriptSource::File(file.to_string()),
                        content,
                    }),
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
                            expr_count += 1;
                            let rest: String = chars[j + 1..].iter().collect();
                            if !rest.is_empty() {
                                opts.scripts.push(ScriptEntry {
                                    source: ScriptSource::Expression(expr_count),
                                    content: rest,
                                });
                            } else {
                                i += 1;
                                if i < args.len() {
                                    opts.scripts.push(ScriptEntry {
                                        source: ScriptSource::Expression(expr_count),
                                        content: args[i].clone(),
                                    });
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
                                    Ok(content) => opts.scripts.push(ScriptEntry {
                                        source: ScriptSource::File(file.clone()),
                                        content,
                                    }),
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
                        'l' => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let n = if !rest.is_empty() {
                                rest.parse().unwrap_or(70)
                            } else {
                                i += 1;
                                if i < args.len() {
                                    args[i].parse().unwrap_or(70)
                                } else {
                                    70
                                }
                            };
                            opts.line_length = n;
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
                if opts.scripts.is_empty() {
                    expr_count += 1;
                    opts.scripts.push(ScriptEntry {
                        source: ScriptSource::Expression(expr_count),
                        content: args[i].clone(),
                    });
                } else {
                    opts.files.push(args[i].clone());
                }
                i += 1;
            }
        }
    }

    if opts.scripts.is_empty() {
        print_usage(&mut io::stderr(), false);
        process::exit(1);
    }

    // COLS env var sets default line width (overridden by -l flag)
    // GNU sed uses COLS - 1 as the wrap width
    if opts.line_length == 70 {
        // Only apply COLS if -l wasn't explicitly set
        if let Ok(cols) = std::env::var("COLS") {
            if let Ok(n) = cols.parse::<usize>() {
                if n > 1 {
                    opts.line_length = n - 1;
                }
                // COLS=0 or COLS=1 → use default
            }
        }
    }

    opts
}

fn main() {
    let opts = parse_options();

    let mut commands = Vec::new();
    let mut hash_n_quiet = false;
    for (idx, script) in opts.scripts.iter().enumerate() {
        let mut parser = Parser::new(&script.content, opts.extended, script.source.clone());
        parser.sandbox = opts.sandbox;
        parser.posix = opts.posix; // only --posix flag, not POSIXLY_CORRECT
        match parser.parse_all(idx == 0) {
            Ok(cmds) => {
                if idx == 0 && parser.hash_n_quiet {
                    hash_n_quiet = true;
                }
                commands.extend(cmds);
            }
            Err(e) => {
                eprintln!("sed: {e}");
                process::exit(1); // EXIT_BAD_INPUT — matches GNU sed
            }
        }
    }

    let quiet = opts.quiet || hash_n_quiet;
    let posix = opts.posix || std::env::var("POSIXLY_CORRECT").is_ok();

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
                    eprintln!("sed: {file}: {}", fmt_io_err(&e));
                    continue;
                }
            };

            if !suffix.is_empty() {
                let backup = if suffix.contains('*') {
                    suffix.replace('*', file)
                } else {
                    format!("{file}{suffix}")
                };
                if let Err(e) = std::fs::copy(file, &backup) {
                    eprintln!("sed: cannot rename {file}: {}", fmt_io_err(&e));
                    process::exit(4);
                }
            }

            // Each file gets a fresh engine in in-place mode
            let mut engine = Engine::new(commands.clone(), quiet, posix, opts.sandbox, opts.line_length);
            engine.current_filename = Some(file.clone());
            let reader = io::BufReader::new(content.as_bytes());
            let mut output = Vec::new();
            let code = engine.run(reader, &mut output).unwrap_or_else(|e| {
                eprintln!("sed: {file}: {}", fmt_io_err(&e));
                1
            });

            if let Err(e) = std::fs::write(file, &output) {
                eprintln!("sed: {file}: {}", fmt_io_err(&e));
            }

            if code != 0 {
                process::exit(code);
            }
        }
    } else if opts.files.is_empty() || (opts.files.len() == 1 && opts.files[0] == "-") {
        let stdin = io::stdin();
        let reader = stdin.lock();
        let mut engine = Engine::new(commands, quiet, posix, opts.sandbox, opts.line_length);
        let code = engine.run(reader, &mut out).unwrap_or_else(|e| {
            eprintln!("sed: {e}");
            1
        });
        process::exit(code);
    } else {
        let mut engine = Engine::new(commands, quiet, posix, opts.sandbox, opts.line_length);
        for file in &opts.files {
            let content = if file == "-" {
                let mut buf = String::new();
                io::stdin().read_to_string(&mut buf).unwrap_or_default();
                buf
            } else {
                match std::fs::read_to_string(file) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("sed: {file}: {}", fmt_io_err(&e));
                        continue;
                    }
                }
            };

            engine.current_filename = Some(file.clone());
            let reader = io::BufReader::new(content.as_bytes());
            if let Err(e) = engine.run(reader, &mut out) {
                eprintln!("sed: {file}: {}", fmt_io_err(&e));
            }
        }
    }
}
