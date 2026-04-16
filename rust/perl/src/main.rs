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
    clippy::useless_format
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
    let args: Vec<String> = env::args().collect();
    let mut program_text = String::new();
    let mut script_file = String::new();
    let mut include_dirs: Vec<String> = Vec::new();

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
            s if s.starts_with("-e") => {
                if !program_text.is_empty() {
                    program_text.push('\n');
                }
                program_text.push_str(&s[2..]);
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
                // Warnings — ignore for now
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
        match fs::read_to_string(&script_file) {
            Ok(content) => program_text = content,
            Err(e) => {
                eprintln!("Can't open perl script \"{}\": {}", script_file, e);
                std::process::exit(2);
            }
        }
    }

    if program_text.is_empty() {
        eprintln!("usage: perl [switches] [programfile] [arguments]");
        std::process::exit(1);
    }

    // Strip shebang line
    if program_text.starts_with("#!") {
        if let Some(newline) = program_text.find('\n') {
            program_text = program_text[newline + 1..].to_string();
        }
    }

    // Tokenize
    let mut lexer = Lexer::new(&program_text);
    let tokens = lexer.tokenize();

    // Parse
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program();

    // Execute
    let mut interp = Interpreter::new();
    interp.run(&program);

    std::process::exit(interp.exit_code);
}
