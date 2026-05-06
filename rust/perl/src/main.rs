#![allow(
    dead_code,
    unused_variables,
    clippy::needless_return,
    clippy::collapsible_if,
    clippy::upper_case_acronyms,
    clippy::never_loop,
    clippy::single_match,
    clippy::manual_map,
    clippy::match_like_matches_macro,
    clippy::redundant_field_names,
    clippy::ptr_arg,
    clippy::len_zero,
    clippy::comparison_chain,
    clippy::manual_range_contains,
    clippy::single_char_pattern,
    clippy::unnecessary_to_owned,
    clippy::clone_on_copy,
    clippy::needless_range_loop,
    clippy::map_clone,
    clippy::useless_format,
    clippy::manual_strip,
    clippy::unnecessary_sort_by,
    clippy::collapsible_else_if,
    clippy::needless_late_init
)]

mod ast;
mod interpreter;
mod lexer;
mod parser;
mod value;

use std::env;
use std::fs;

use interpreter::Interpreter;
use lexer::Lexer;
use parser::Parser;

fn main() {
    // Run the interpreter in a thread with a larger stack than Rust's default
    // main-thread stack (typically 8 MiB, but the interpreter is recursive
    // and Perl tests like op/cond.t build deeply-nested expressions — e.g.
    // `$x ? 1 : $x ? 1 : …` 20000 levels deep, and op/list.t does `() =
    // (1,(1,(1,…)))` 100 000 levels deep). A 1 GiB thread stack is
    // generous but keeps the cost to at-most one allocation per process.
    let handle = std::thread::Builder::new()
        .stack_size(1024 * 1024 * 1024)
        .spawn(run_interpreter)
        .expect("failed to spawn interpreter thread");
    let code = handle.join().expect("interpreter thread panicked");
    std::process::exit(code);
}

fn run_interpreter() -> i32 {
    let args: Vec<String> = env::args().collect();
    let mut program_text = String::new();
    let mut script_file = String::new();
    let mut include_dirs: Vec<String> = Vec::new();
    let mut auto_newline = false; // -l flag
    let mut warnings_flag = false; // -w flag

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-v" => {
                println!("perl (rust-perl) {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-e" | "-E" => {
                i += 1;
                if i < args.len() {
                    if !program_text.is_empty() {
                        program_text.push('\n');
                    }
                    program_text.push_str(&args[i]);
                }
            }
            s if s.starts_with("-") && s.contains('e') && !s.starts_with("-I") && s.len() > 1 => {
                // Handle combined flags like -le, -lwe, -wle, etc.
                // Everything after -e is the program text (if any), otherwise next arg
                let e_pos = s.find('e').unwrap();
                let after_e = &s[e_pos + 1..];
                if !after_e.is_empty() {
                    if !program_text.is_empty() {
                        program_text.push('\n');
                    }
                    program_text.push_str(after_e);
                } else {
                    i += 1;
                    if i < args.len() {
                        if !program_text.is_empty() {
                            program_text.push('\n');
                        }
                        program_text.push_str(&args[i]);
                    }
                }
                // Parse flags before -e
                let before_e = &s[1..e_pos];
                if before_e.contains('l') {
                    auto_newline = true;
                }
            }
            "-I" => {
                i += 1;
                if i < args.len() {
                    include_dirs.push(args[i].clone());
                }
            }
            s if s.starts_with("-I") => {
                include_dirs.push(s[2..].to_string());
            }
            "-w" | "-W" => {
                warnings_flag = true;
            }
            "-l" => {
                auto_newline = true;
            }
            s if s.starts_with("-")
                && s.len() > 1
                && program_text.is_empty()
                && script_file.is_empty() =>
            {
                // Unknown flag, skip
            }
            _ => {
                if program_text.is_empty() && script_file.is_empty() {
                    script_file = args[i].clone();
                }
            }
        }
        i += 1;
    }

    if !script_file.is_empty() {
        if script_file == "-" {
            // Read program from stdin. Reference perl reports the file
            // label as `-` for diagnostics, which we keep here so error
            // messages match byte-for-byte under fresh_perl etc.
            use std::io::Read;
            let mut bytes = Vec::new();
            if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
                eprintln!("Can't read perl script from stdin: {}", e);
                std::process::exit(2);
            }
            program_text = String::from_utf8_lossy(&bytes).into_owned();
        } else {
            match fs::read(&script_file) {
                // Perl scripts are traditionally byte-oriented — some tests
                // ship Latin-1 content. Decode lossily so the lexer always
                // sees valid UTF-8 (invalid bytes become U+FFFD).
                Ok(bytes) => {
                    program_text = String::from_utf8_lossy(&bytes).into_owned();
                }
                Err(e) => {
                    eprintln!("Can't open perl script \"{}\": {}", script_file, e);
                    std::process::exit(2);
                }
            }
        }
    }

    if program_text.is_empty() {
        eprintln!("usage: perl [switches] [programfile] [arguments]");
        std::process::exit(1);
    }

    // Blank out the shebang line so it's ignored but keeps line numbers
    // accurate. Lexer treats the resulting empty line as whitespace.
    if program_text.starts_with("#!") {
        if let Some(newline) = program_text.find('\n') {
            let rest = program_text[newline + 1..].to_string();
            program_text = format!("\n{rest}");
        }
    }

    // Tokenize
    let mut lexer = Lexer::new(&program_text);
    let tokens = lexer.tokenize();
    let token_lines = std::mem::take(&mut lexer.token_lines);
    let file_overrides = std::mem::take(&mut lexer.file_overrides);

    if let Some(err) = lexer.error.take() {
        let file_label = if script_file.is_empty() {
            "-e"
        } else {
            script_file.as_str()
        };
        eprint!("{}", err.replace("{FILE}", file_label));
        return 255;
    }

    // Parse
    let mut parser = Parser::new_with_lines_and_files(tokens, token_lines, file_overrides);
    let program = parser.parse_program();

    // Execute
    let mut interp = Interpreter::new();
    if auto_newline {
        // -l flag: set $\ (output record separator) to \n
        // and auto-chomp on input
        interp.set_special_var("\\", "\n");
    }
    if warnings_flag {
        interp.enable_warnings();
    }
    if !script_file.is_empty() {
        interp.set_current_file(&script_file);
    }
    // Populate @INC from -I flags (prepended in order).
    interp.set_inc(&include_dirs);
    interp.run(&program);

    interp.exit_code
}
