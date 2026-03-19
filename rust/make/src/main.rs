mod ast;
mod engine;
mod expand;
mod parser;

use engine::Engine;

fn main() {
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let mut engine = Engine::new();
    let mut targets = Vec::new();
    let mut makefile: Option<String> = None;
    let mut directory: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-f" | "--file" | "--makefile" => {
                i += 1;
                if i < args.len() {
                    makefile = Some(args[i].clone());
                }
            }
            "-C" | "--directory" => {
                i += 1;
                if i < args.len() {
                    directory = Some(args[i].clone());
                }
            }
            "-j" | "--jobs" => {
                i += 1;
                if i < args.len() {
                    engine.jobs = args[i].parse().unwrap_or(1);
                }
            }
            "-n" | "--just-print" | "--dry-run" | "--recon" => {
                engine.dry_run = true;
            }
            "-s" | "--silent" | "--quiet" => {
                engine.silent = true;
            }
            "-k" | "--keep-going" => {
                engine.keep_going = true;
            }
            "-t" | "--touch" => {
                engine.touch = true;
            }
            "-q" | "--question" => {
                engine.question = true;
            }
            "-B" | "--always-make" => {
                engine.always_make = true;
            }
            "-i" | "--ignore-errors" => {
                // Ignore errors in all recipes
            }
            "-w" | "--print-directory" => {
                let cwd = std::env::current_dir().unwrap_or_default();
                eprintln!("make: Entering directory '{}'", cwd.display());
            }
            "--no-print-directory" => {}
            "-p" | "--print-data-base" => {
                // TODO: print database
            }
            "-v" | "--version" => {
                println!("GNU Make 0.1.0-rust (compatible)");
                println!("This is rust-make, a GNU Make-compatible build system.");
                return 0;
            }
            "-h" | "--help" => {
                println!("Usage: make [options] [target] ...");
                println!("Options:");
                println!("  -f FILE  Read FILE as a makefile");
                println!("  -C DIR   Change to DIR before doing anything");
                println!("  -j N     Allow N jobs at once");
                println!("  -n       Dry run (print commands without executing)");
                println!("  -s       Silent mode");
                println!("  -k       Keep going on errors");
                println!("  -t       Touch targets instead of building");
                println!("  -q       Question mode (exit 1 if not up to date)");
                println!("  -B       Always make all targets");
                return 0;
            }
            arg if arg.starts_with("-j") => {
                engine.jobs = arg[2..].parse().unwrap_or(1);
            }
            arg if arg.starts_with("-f") => {
                makefile = Some(arg[2..].to_string());
            }
            arg if arg.starts_with("-C") => {
                directory = Some(arg[2..].to_string());
            }
            arg if arg.contains('=') => {
                // Command-line variable assignment
                if let Some(eq_pos) = arg.find('=') {
                    let name = &arg[..eq_pos];
                    let value = &arg[eq_pos + 1..];
                    engine.set_var_with_origin(
                        name,
                        value,
                        engine::VarFlavor::Simple,
                        engine::VarOrigin::CommandLine,
                    );
                }
            }
            arg if arg.starts_with('-') => {
                // Handle combined short flags like -ks
                let flags = &arg[1..];
                for flag in flags.chars() {
                    match flag {
                        'n' => engine.dry_run = true,
                        's' => engine.silent = true,
                        'k' => engine.keep_going = true,
                        't' => engine.touch = true,
                        'q' => engine.question = true,
                        'B' => engine.always_make = true,
                        'i' => {} // ignore errors
                        'w' => {}
                        _ => {
                            eprintln!("make: Unknown option '-{flag}'");
                        }
                    }
                }
            }
            _ => {
                targets.push(args[i].clone());
            }
        }
        i += 1;
    }

    // Change directory if requested
    if let Some(dir) = directory
        && let Err(e) = std::env::set_current_dir(&dir)
    {
        eprintln!("make: *** {dir}: {e}.  Stop.");
        return 2;
    }

    // Find and load makefile
    let makefile_path = if let Some(f) = makefile {
        f
    } else if std::path::Path::new("GNUmakefile").exists() {
        "GNUmakefile".to_string()
    } else if std::path::Path::new("makefile").exists() {
        "makefile".to_string()
    } else if std::path::Path::new("Makefile").exists() {
        "Makefile".to_string()
    } else {
        eprintln!("make: *** No makefile found.  Stop.");
        return 2;
    };

    engine.load_file(&makefile_path, false);

    engine.build(&targets)
}
