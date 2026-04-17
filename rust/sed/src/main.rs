mod engine;
mod parser;
mod regex_util;
mod types;
mod util;

#[allow(unused_imports)]
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
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

fn resolve_symlinks(path: &str) -> String {
    let mut current = std::path::PathBuf::from(path);
    for _ in 0..40 {
        // limit to prevent infinite loops
        match std::fs::read_link(&current) {
            Ok(target) => {
                if target.is_relative() {
                    if let Some(parent) = current.parent() {
                        current = parent.join(&target);
                    } else {
                        current = target;
                    }
                } else {
                    current = target;
                }
            }
            Err(_) => break, // not a symlink or error
        }
    }
    current.to_string_lossy().into_owned()
}

fn collect_labels(commands: &[types::SedCommand], labels: &mut std::collections::HashSet<String>) {
    for cmd in commands {
        if let types::Command::Label(l) = &cmd.command {
            labels.insert(l.clone());
        }
        if let types::Command::Block(inner) = &cmd.command {
            collect_labels(inner, labels);
        }
    }
}

fn collect_branches(commands: &[types::SedCommand], branches: &mut Vec<String>) {
    for cmd in commands {
        match &cmd.command {
            types::Command::Branch(Some(l))
            | types::Command::BranchIfSub(Some(l))
            | types::Command::BranchIfNoSub(Some(l)) => {
                branches.push(l.clone());
            }
            types::Command::Block(inner) => {
                collect_branches(&inner, branches);
            }
            _ => {}
        }
    }
}

fn validate_labels(commands: &[types::SedCommand]) {
    let mut labels = std::collections::HashSet::new();
    let mut branches = Vec::new();
    collect_labels(commands, &mut labels);
    collect_branches(commands, &mut branches);
    for label in &branches {
        if !labels.contains(label) {
            eprintln!("sed: can't find label for jump to `{label}'");
            process::exit(1);
        }
    }
}

fn read_script_file(path: &str) -> Result<String, String> {
    let bytes = if path == "-" {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| format!("sed: -: {}", fmt_io_err(&e)))?;
        buf
    } else {
        std::fs::read(path).map_err(|e| format!("sed: {path}: {}", fmt_io_err(&e)))?
    };
    Ok(bytes_to_string_latin1(&bytes))
}

/// Decode bytes as UTF-8 if valid, else as Latin-1 (byte-preserving 1:1).
/// This keeps non-UTF-8 bytes in sed scripts addressable as single chars,
/// so replacements can reproduce the original byte via the Latin-1 output path.
fn bytes_to_string_latin1(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
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
        follow_symlinks: false,
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
                opts.follow_symlinks = true;
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

    if opts.posix {
        // POSIX mode: parse each expression separately (for incomplete command detection)
        for (idx, script) in opts.scripts.iter().enumerate() {
            let mut parser = Parser::new(&script.content, opts.extended, script.source.clone());
            parser.sandbox = opts.sandbox;
            parser.posix = true;
            parser.is_last_script = idx == opts.scripts.len() - 1;
            match parser.parse_all(idx == 0) {
                Ok(cmds) => {
                    if idx == 0 && parser.hash_n_quiet {
                        hash_n_quiet = true;
                    }
                    commands.extend(cmds);
                }
                Err(e) => {
                    eprintln!("sed: {e}");
                    process::exit(1);
                }
            }
        }
    } else {
        // GNU mode: join all expressions with newlines (allows a/c/i continuation)
        let combined: String = opts.scripts.iter().map(|s| s.content.clone()).collect::<Vec<_>>().join("\n");
        let source = opts.scripts.first().map(|s| s.source.clone())
            .unwrap_or(ScriptSource::Expression(1));
        let mut parser = Parser::new(&combined, opts.extended, source);
        parser.sandbox = opts.sandbox;
        parser.posix = false;
        parser.is_last_script = true;
        match parser.parse_all(true) {
            Ok(cmds) => {
                if parser.hash_n_quiet {
                    hash_n_quiet = true;
                }
                commands.extend(cmds);
            }
            Err(e) => {
                eprintln!("sed: {e}");
                process::exit(1);
            }
        }
    }

    // Validate branch targets
    validate_labels(&commands);

    let quiet = opts.quiet || hash_n_quiet;
    let posix = opts.posix || std::env::var("POSIXLY_CORRECT").is_ok();

    let stdout = io::stdout();
    let mut out = stdout.lock();

    if opts.in_place.is_some() {
        let suffix = opts.in_place.as_deref().unwrap_or("");
        if opts.files.is_empty() {
            eprintln!("sed: no input files");
            process::exit(4);
        }
        for file in &opts.files {
            // Check file type for in-place editing
            if let Ok(meta) = std::fs::metadata(file) {
                if !meta.is_file() {
                    if meta.file_type().is_fifo() || meta.file_type().is_char_device() {
                        let kind = if meta.file_type().is_char_device() {
                            "is a terminal"
                        } else {
                            "not a regular file"
                        };
                        eprintln!("sed: couldn't edit {file}: {kind}");
                        process::exit(4);
                    }
                }
            }

            let actual_read = if opts.follow_symlinks {
                resolve_symlinks(file)
            } else {
                file.clone()
            };
            let raw_content = match std::fs::read(&actual_read) {
                Ok(bytes) => bytes,
                Err(e) => {
                    if opts.follow_symlinks
                        && (!std::path::Path::new(file).exists()
                            || e.raw_os_error() == Some(40))
                    {
                        let msg = fmt_io_err(&e);
                        if msg.is_empty() {
                            eprintln!("sed: couldn't readlink {file}:");
                        } else {
                            eprintln!("sed: couldn't readlink {file}: {msg}");
                        }
                        process::exit(4);
                    }
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

            // Check if we can write to the file's directory (for temp file)
            if let Some(parent) = std::path::Path::new(file).parent() {
                let parent = if parent.as_os_str().is_empty() {
                    std::path::Path::new(".")
                } else {
                    parent
                };
                // Try to create a temp file in the directory to check writeability
                let tmp_path = parent.join(format!(".sed-tmp-{}", std::process::id()));
                match std::fs::File::create(&tmp_path) {
                    Ok(_) => {
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                    Err(e) => {
                        eprintln!("sed: couldn't open temporary file {}: {}",
                            tmp_path.display(), fmt_io_err(&e));
                        process::exit(4);
                    }
                }
            }

            // Each file gets a fresh engine in in-place mode
            let mut engine = Engine::new(commands.clone(), quiet, posix, opts.sandbox, opts.line_length, opts.null_data);
            engine.current_filename = Some(if opts.follow_symlinks {
                resolve_symlinks(file)
            } else {
                file.clone()
            });
            let reader = io::BufReader::new(raw_content.as_slice());
            let mut output = Vec::new();
            let code = engine.run(reader, &mut output).unwrap_or_else(|e| {
                eprintln!("sed: {file}: {}", fmt_io_err(&e));
                1
            });

            if let Err(e) = std::fs::write(file, &output) {
                let msg = fmt_io_err(&e);
                if e.kind() == io::ErrorKind::PermissionDenied {
                    eprintln!("sed: couldn't open temporary file {file}: {msg}");
                } else {
                    eprintln!("sed: {file}: {msg}");
                }
                process::exit(4);
            }

            if code != 0 {
                process::exit(code);
            }
        }
    } else if opts.files.is_empty() || (opts.files.len() == 1 && opts.files[0] == "-") {
        let stdin = io::stdin();
        let mut engine = Engine::new(commands, quiet, posix, opts.sandbox, opts.line_length, opts.null_data);
        let code = if opts.unbuffered {
            // Read byte-by-byte from raw fd 0 to avoid read-ahead buffering.
            // This ensures that after sed exits (e.g., via `q`), unconsumed
            // input remains available for the next process in the pipeline.
            use std::os::unix::io::FromRawFd;
            let raw = unsafe { std::fs::File::from_raw_fd(0) };
            let reader = io::BufReader::with_capacity(1, raw);
            engine.run_unbuffered(reader, &mut out)
            // File closes fd 0 on drop, but we're about to exit anyway
        } else {
            let reader = stdin.lock();
            engine.run(reader, &mut out)
        }.unwrap_or_else(|e| {
            eprintln!("sed: {e}");
            1
        });
        process::exit(code);
    } else {
        let mut engine = Engine::new(commands, quiet, posix, opts.sandbox, opts.line_length, opts.null_data);
        let mut exit_code = 0i32;
        // Determine the last file with actual content for $ detection
        let last_content_idx = {
            let mut last = opts.files.len().saturating_sub(1);
            for i in (0..opts.files.len()).rev() {
                let f = &opts.files[i];
                if f == "-" || std::fs::metadata(f).map(|m| m.len() > 0).unwrap_or(false) {
                    last = i;
                    break;
                }
            }
            last
        };
        for (file_idx, file) in opts.files.iter().enumerate() {
            engine.is_last_file = file_idx >= last_content_idx;
            let raw_content = if file == "-" {
                let mut buf = Vec::new();
                io::stdin().read_to_end(&mut buf).unwrap_or_default();
                buf
            } else {
                let actual_file = if opts.follow_symlinks {
                    resolve_symlinks(file)
                } else {
                    file.clone()
                };
                match std::fs::read(&actual_file) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        if opts.follow_symlinks {
                            let msg = fmt_io_err(&e);
                            // Non-existent file or symlink loop
                            if !std::path::Path::new(file).exists()
                                || e.raw_os_error() == Some(40)
                            {
                                // 40 = ELOOP on Linux
                                if msg.is_empty() {
                                    eprintln!("sed: couldn't readlink {file}:");
                                } else {
                                    eprintln!("sed: couldn't readlink {file}: {msg}");
                                }
                                process::exit(4);
                            }
                        }
                        eprintln!("sed: {file}: {}", fmt_io_err(&e));
                        exit_code = 2;
                        continue;
                    }
                }
            };

            engine.current_filename = Some(if opts.follow_symlinks {
                resolve_symlinks(file)
            } else {
                file.clone()
            });
            let reader = io::BufReader::new(raw_content.as_slice());
            match engine.run(reader, &mut out) {
                Ok(code) if code != 0 => process::exit(code),
                Err(e) => {
                    eprintln!("sed: {file}: {}", fmt_io_err(&e));
                    exit_code = 2;
                }
                _ => {}
            }
        }
        if exit_code != 0 {
            process::exit(exit_code);
        }
    }
}
